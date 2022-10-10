mod simulator;

use clap::Parser;
use num_bigint::{BigInt, BigUint, Sign};
use regex::Regex;
use scry_asm::Assemble;
use scry_sim::{
	BlockedMemory, CallFrameState, ExecError, ExecState, Executor, Metric, MetricReporter,
	OperandList, OperandState, Scalar, TrackReport, Value, ValueType,
};
use std::{collections::HashMap, time::Instant};

#[derive(clap::ValueEnum, Clone, Eq, PartialEq)]
enum TimeoutType
{
	Instructions,
	Seconds,
}

/// Command-line arguments
#[derive(Parser)]
struct Cli
{
	/// The path to the file to execute
	path: std::path::PathBuf,

	/// For when the simulator needs to emulate the program exactly.
	/// In this mode, the simulator's outputs (exit code, stdout, stderr) come
	/// from the simulated program.
	#[clap(short, long)]
	machine_mode: bool,

	/// Signals that the file is a binary assembly file (i.e. not text
	/// assembly).
	#[clap(short, long)]
	binary: bool,

	/// Input operand to the first instruction.
	/// Can be given multiple times for multiple input operands.
	#[clap(short, long)]
	input: Vec<String>,

	/// Stop the simulation early.
	#[clap(long)]
	#[arg(default_value_t = 0)]
	timeout: usize,

	/// What counter should be used to timeout.
	#[clap(long)]
	#[arg(value_enum, default_value_t = TimeoutType::Seconds)]
	timeout_type: TimeoutType,

	#[clap(long)]
	debug: bool,
}

fn parse_input(input: &String) -> OperandState<usize>
{
	let re = Regex::new(r"^(-?\d+)(u|i)(\d+)$").unwrap();
	let caps = re.captures_iter(input.as_str()).next().unwrap();

	let byte_size_pow_2: u8 = caps[3].parse().unwrap();
	let byte_size = 1 << byte_size_pow_2;
	let signed = &caps[2] == "i";
	let typ = if signed
	{
		ValueType::Int(byte_size_pow_2)
	}
	else
	{
		ValueType::Uint(byte_size_pow_2)
	};
	let (sign, mut value_bytes) = if signed
	{
		let value = BigInt::parse_bytes(&caps[1].as_bytes(), 10).unwrap();
		(value.sign(), value.to_signed_bytes_le())
	}
	else
	{
		(
			Sign::Plus,
			BigUint::parse_bytes(caps[1].as_bytes(), 10)
				.unwrap()
				.to_bytes_le(),
		)
	};
	// Ensure the number of bytes fits the type
	value_bytes.resize(
		byte_size as usize,
		if sign == Sign::Minus { u8::MAX } else { 0 },
	);
	assert_eq!(value_bytes.len(), byte_size as usize);

	OperandState::Ready(Value::singleton_typed(
		typ,
		Scalar::Val(value_bytes.into_boxed_slice()),
	))
}

fn operand_to_string(op: &OperandState<usize>) -> String
{
	if let OperandState::Ready(val) = op
	{
		let (mut val, typ, size_pow_2): (String, char, u8) = match val.value_type()
		{
			ValueType::Uint(x) =>
			{
				let int = BigUint::from_bytes_le(val.iter().next().unwrap().bytes().unwrap());
				(int.to_string(), 'u', x)
			},
			ValueType::Int(x) =>
			{
				let int = BigInt::from_signed_bytes_le(val.iter().next().unwrap().bytes().unwrap());
				(int.to_string(), 'i', x)
			},
		};
		val.push(typ);
		val.push_str(&*size_pow_2.to_string());
		val
	}
	else
	{
		todo!()
	}
}

fn print_metrics(tracker: &TrackReport)
{
	println!("\n----------  Simulation Metrics  ----------");
	use scry_sim::Metric::*;
	for metric in [
		IssuedBranches,
		IssuedCalls,
		IssuedReturns,
		TriggeredBranches,
		TriggeredCalls,
		TriggeredReturns,
		ConsumedOperands,
		ConsumedBytes,
		QueuedValues,
		QueuedValueBytes,
		QueuedReads,
		ReorderedOperands,
		InstructionReads,
		DataReads,
		DataBytesRead,
		DataBytesWritten,
		UnalignedReads,
		UnalignedWrites,
	]
	{
		let metric_val = tracker.get_stat(metric);
		println!("{:?}: {}", metric, metric_val);
	}
}

fn main()
{
	let start_time = Instant::now();

	let args = Cli::parse();

	let contents = std::fs::read(args.path).unwrap();

	let program = if args.binary
	{
		contents
	}
	else
	{
		// File is in textual assembly, assemble it
		scry_asm::Raw::assemble(std::iter::once(
			String::from_utf8(contents).unwrap().as_str(),
		))
		.unwrap()
	};

	// Ready inputs
	let mut op_queue = HashMap::new();
	if !args.input.is_empty()
	{
		let mut ops: Vec<_> = args.input.iter().map(|s| parse_input(s)).collect();
		op_queue.insert(0, OperandList::new(ops.remove(0), ops));
	}

	let original_state = ExecState {
		address: 0,
		frame: CallFrameState {
			ret_addr: 0,
			branches: HashMap::new(),
			op_queue,
			reads: Vec::new(),
		},
		frame_stack: vec![CallFrameState {
			ret_addr: 0,
			branches: HashMap::new(),
			op_queue: HashMap::new(),
			reads: Vec::new(),
		}],
	};
	let mut tracker = TrackReport::new();
	if args.debug
	{
		dbg!(&original_state);
	}
	let mut res =
		Executor::from_state(&original_state, BlockedMemory::new(program, 0)).step(&mut tracker);
	while res.is_ok()
	{
		let exec = res.unwrap();
		let state = exec.state();
		if args.debug
		{
			dbg!(&state);
		}

		if state.frame_stack.len() == 0
		{
			// Done
			match state.frame.op_queue.get(&0)
			{
				Some(ready_list) =>
				{
					if let OperandState::Ready(v) = &ready_list.first
					{
						if args.machine_mode
						{
							std::process::exit(
								v.iter()
									.next()
									.unwrap()
									.bytes()
									.map_or(123, |b| b[0] as i32),
							);
						}
						else
						{
							println!("----------  Returned Operands  ----------");
							for op in ready_list.iter()
							{
								print!("{}, ", operand_to_string(op));
							}

							print_metrics(&tracker);

							// Success
							return;
						}
					}
				},
				_ => (),
			}
			// Failure
			res = Err(ExecError::Err);
			continue;
		}

		if args.timeout > 0
			&& ((args.timeout_type == TimeoutType::Instructions
				&& tracker.get_stat(Metric::InstructionReads) == args.timeout)
				| (args.timeout_type == TimeoutType::Seconds
					&& start_time.elapsed().as_secs() > args.timeout as u64))
		{
			if !args.machine_mode
			{
				println!("----------  Timeout  ----------");
				print_metrics(&tracker);
			}
			std::process::exit(123)
		}

		res = exec.step(&mut tracker);
	}
	// Implicit failure
	match res
	{
		Err(err) =>
		{
			if !args.machine_mode
			{
				println!("----------  Error  ----------");
				println!("{:?}", err);
				print_metrics(&tracker);
			}
		},
		Ok(_) => unreachable!(),
	}
	std::process::exit(123)
}
