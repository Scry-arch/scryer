mod simulator;

use clap::Parser;
use num_bigint::{BigInt, BigUint, Sign};
use regex::Regex;
use scry_asm::Assemble;
use scry_sim::{
	BlockedMemory, CallFrameState, ExecError, ExecState, Executor, MemError, Memory, Metric,
	MetricReporter, OperandList, OperandState, Scalar, TrackReport, Value, ValueType,
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

fn operand_to_value(
	op: &OperandState<usize>,
	reads: &Vec<(usize, usize, ValueType)>,
	memory: &mut impl Memory,
) -> Result<Value, (MemError, usize)>
{
	match op
	{
		OperandState::Ready(val) => Ok(val.clone()),
		OperandState::MustRead(idx) =>
		{
			if let Some((addr, count, typ)) = reads.get(*idx)
			{
				assert!(*count == 1);
				let mut val = Value::new_nan_typed(typ.clone());
				memory.read_data(*addr, &mut val, 1, &mut ())?;
				Ok(val)
			}
			else
			{
				panic!("Issued load doesn't exist in list");
			}
		},
	}
}

fn value_to_string(val: Value) -> String
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
		DataReadBytes,
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
	let mut memory = BlockedMemory::new(program.into_iter(), 0);
	let mut res =
		Executor::<BlockedMemory, _>::from_state(&original_state, &mut memory).step(&mut tracker);
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
			if let Some(ready_list) = state.frame.op_queue.get(&0)
			{
				let mut returned_values = Vec::new();

				// Extract Values from all operands, both to ensure we can and to pretty print
				for op in ready_list.iter()
				{
					returned_values
						.push(operand_to_value(op, &state.frame.reads, &mut memory).unwrap());
				}

				if args.machine_mode
				{
					// Return the integer value of the first return operand or 123 if unavailable
					std::process::exit(
						returned_values
							.iter()
							.next()
							.unwrap()
							.get_first()
							.bytes()
							.map_or(123, |b| b[0] as i32),
					);
				}
				else
				{
					// Pretty print the returned operands
					println!("----------  Returned Operands  ----------");
					for (val, op) in returned_values.into_iter().zip(ready_list.iter())
					{
						let val_str = value_to_string(val);
						if let OperandState::MustRead(idx) = op
						{
							let addr = state.frame.reads[*idx].0;
							print!("Load({:#X},{}), ", addr, val_str);
						}
						else
						{
							print!("{}, ", val_str);
						}
					}

					print_metrics(&tracker);
					return;
				}
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
