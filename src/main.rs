mod simulator;

use clap::Parser;
use num_bigint::Sign;
use scry_asm::Assemble;
use scry_sim::{
	BlockedMemory, CallFrameState, ExecState, Executor, OperandState, Scalar, Value, ValueType,
};
use std::collections::HashMap;

/// Command-line arguments
#[derive(Parser)]
struct Cli
{
	/// The path to the file to execute
	#[clap(parse(from_os_str))]
	path: std::path::PathBuf,

	/// Signals that the file is a binary assembly file (i.e. not text
	/// assembly).
	#[clap(short, long)]
	binary: bool,

	/// Input operand to the first instruction.
	/// Can be given multiple times for multiple input operands.
	#[clap(short, long)]
	input: Vec<String>,
}

fn parse_input(input: &String) -> OperandState<usize>
{
	use num_bigint::{BigInt, BigUint};
	use regex::Regex;

	let re = Regex::new(r"^(-?\d+)([u|i])(\d+)$").unwrap();
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

fn main()
{
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
	let mut op_queues = HashMap::new();
	if !args.input.is_empty()
	{
		let mut ops: Vec<_> = args.input.iter().map(|s| parse_input(s)).collect();
		op_queues.insert(0, (ops.remove(0), ops));
	}

	let original_state = ExecState {
		address: 0,
		frame: CallFrameState {
			ret_addr: 0,
			branches: HashMap::new(),
			op_queues,
			reads: Vec::new(),
		},
		frame_stack: vec![CallFrameState {
			ret_addr: 0,
			branches: HashMap::new(),
			op_queues: HashMap::new(),
			reads: Vec::new(),
		}],
	};
	let mut res =
		Executor::from_state(&original_state, BlockedMemory::new(program, 0)).step(&mut ());
	while res.is_ok()
	{
		let exec = res.unwrap();
		let state = exec.state();
		if state.frame_stack.len() == 0
		{
			// Done
			if let Some((OperandState::Ready(v), _)) = state.frame.op_queues.get(&0)
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
				std::process::exit(123);
			}
		}
		res = exec.step(&mut ());
	}
}
