use crate::events::StepTracker;
use clap::Parser;
use num_bigint::{BigInt, BigUint, Sign};
use regex::Regex;
use scry_asm::Disassemble;
use scry_sim::{Event, IdEvent, OperandEvent, Scalar, Value, ValueType};
use std::{collections::HashMap, io::Write};

#[derive(clap::ValueEnum, Clone, Eq, PartialEq)]
pub enum TimeoutType
{
	Instructions,
	Seconds,
}

#[derive(clap::ValueEnum, Clone, Debug, PartialEq, Eq)]
pub enum Target
{
	/// Tries to detect the target automatically from file contents, except
	/// 'raw'.
	Auto,
	/// Raw binary file containing only encoded instructions
	Raw,
	/// File containing textual assembly
	Assembly,
	/// 32-bit ELF file
	Scry32UnknownNoneElf,
	/// 64-bit ELF file
	Scry64UnknownNoneElf,
}

/// Command-line arguments
#[derive(Parser)]
pub struct Cli
{
	/// The path to the file to execute
	pub path: std::path::PathBuf,

	/// For when the simulator needs to emulate the program exactly.
	/// In this mode, the simulator's outputs (exit code, stdout, stderr) come
	/// from the simulated program.
	#[clap(short, long)]
	pub machine_mode: bool,

	/// The target triple of the input file
	#[clap(short, long, value_name = "TRIPLE")]
	#[arg(value_enum, default_value_t = Target::Auto)]
	pub target: Target,

	/// Input operand to the first instruction.
	/// Can be given multiple times for multiple input operands.
	/// Must give value and its type. E.g. '1u8' is an unsigned byte of value 1.
	/// Can give relative to a known label. E.g. 'entry+1u8' is an unsigned byte
	/// of value one higher than the 'entry' label.
	#[clap(short, long)]
	pub input: Vec<String>,

	/// Stop the simulation early.
	#[clap(long)]
	#[arg(default_value_t = 0)]
	pub timeout: usize,

	/// What counter should be used to timeout.
	#[clap(long)]
	#[arg(value_enum, default_value_t = TimeoutType::Seconds)]
	pub timeout_type: TimeoutType,

	/// Prints simulation events either to stdout or a file
	#[clap(long, value_name = "FILE", num_args = 0..=1, require_equals = true)]
	pub print_events: Option<Option<std::path::PathBuf>>,

	/// Event will be buffered and only printing after the simulation terminates
	#[clap(long)]
	#[arg(default_value_t = false)]
	pub print_events_buffered: bool,

	/// Where the initial stack base should be located.
	///
	/// If given an integer (decimal or hexadecimal), the stack base starts at
	/// that address. If given a string label, the corresponding address will be
	/// used.
	#[clap(long, value_name = "ADDRESS")]
	#[arg(default_value_t = String::new())]
	pub stack_base: String,

	/// Initial size of the stack buffer.
	#[clap(long, value_name = "BYTES")]
	#[arg(default_value_t = 1 << 12)]
	pub stack_buffer: usize,
}

pub fn parse_input(input: &String, labels: HashMap<&str, usize>) -> Value
{
	let re = Regex::new(r"^((.+)\+)?(-?\d+)(u|i)(\d+)$").unwrap();
	let caps = re.captures_iter(input.as_str()).next().unwrap();

	let bit_size: u8 = caps[5].parse().unwrap();
	let byte_size = bit_size / 8;
	let byte_size_pow_2 = byte_size.ilog2() as u8;
	let signed = &caps[4] == "i";
	let value = caps[3].as_bytes();
	let relative_label = caps.get(2);
	let add_to_value = || {
		if let Some(label) = relative_label
		{
			if let Some(addr) = labels.get(label.as_str())
			{
				return *addr;
			}
		}
		0
	};

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
		let mut value = BigInt::parse_bytes(&value, 10).unwrap();
		value += add_to_value();
		(value.sign(), value.to_signed_bytes_le())
	}
	else
	{
		let mut value = BigUint::parse_bytes(value, 10).unwrap();
		value += add_to_value();
		(Sign::Plus, value.to_bytes_le())
	};
	// Ensure the number of bytes fits the type
	value_bytes.resize(
		byte_size as usize,
		if sign == Sign::Minus { u8::MAX } else { 0 },
	);
	assert_eq!(value_bytes.len(), byte_size as usize);

	Value::singleton_typed(typ, Scalar::Val(value_bytes.into_boxed_slice()))
}

pub fn value_to_string(val: &Value) -> String
{
	let (mut val, typ, size_pow_2): (String, char, u8) = match val.value_type()
	{
		ValueType::Uint(x) =>
		{
			let int = BigUint::from_bytes_le(val.iter().next().unwrap().bytes().unwrap());
			(int.to_string(), 'u', 2u8.pow(x as u32) * 8)
		},
		ValueType::Int(x) =>
		{
			let int = BigInt::from_signed_bytes_le(val.iter().next().unwrap().bytes().unwrap());
			(int.to_string(), 'i', 2u8.pow(x as u32) * 8)
		},
	};
	val.push(typ);
	val.push_str(&*size_pow_2.to_string());
	val
}

pub fn print_events(tracker: &StepTracker, out: &mut impl Write)
{
	for step_events in tracker.all_events()
	{
		for event in step_events
		{
			match event
			{
				Event::Instruction(addr, instr) =>
				{
					write!(
						out,
						"Instruction: ({:#x}) {}",
						addr,
						scry_asm::Raw::disassemble(instr.encode().to_le_bytes().into_iter())
							.unwrap()
					)
				},
				Event::Id(id, id_typ) =>
				{
					match id_typ
					{
						IdEvent::Operand(OperandEvent::Produce(t)) =>
						{
							write!(out, "Operand: Produce({},", id).unwrap();
							if t.is_signed_int()
							{
								write!(out, "i").unwrap();
							}
							else if t.is_unsigned_int()
							{
								write!(out, "u").unwrap();
							}
							else
							{
								todo!()
							}
							writeln!(out, "{})", t.size() * 8)
						},
						IdEvent::Operand(p) => writeln!(out, "Operand: {:?}({})", p, id),
						IdEvent::ControlIssue(i) => writeln!(out, "Issue: {:?}({})", i, id),
						IdEvent::ControlTrigger(t) => writeln!(out, "Trigger: {:?}({})", t, id),
						_ => writeln!(out, "IdEvent({}): {:?}", id, id_typ),
					}
				},
				Event::Load(addr, idx, bytes) =>
				{
					if let Some(idx) = idx
					{
						writeln!(out, "LoadStack({:#x},{}): {}", addr, idx, bytes)
					}
					else
					{
						writeln!(out, "Load({:#x}): {}", addr, bytes)
					}
				},
				Event::Store(addr, idx, bytes) =>
				{
					if let Some(idx) = idx
					{
						writeln!(out, "StoreStack({:#x},{}): {}", addr, idx, bytes)
					}
					else
					{
						writeln!(out, "Store({:#x}): {}", addr, bytes)
					}
				},
				Event::Stack(section, addr) =>
				{
					writeln!(out, "Stack: {:?}({:#x})", section, addr)
				},
				_ => writeln!(out, "Event: {:?}", event),
			}
			.unwrap();
		}
		writeln!(out, "STEP_END").unwrap();
	}
}

pub fn print_events_final(tracker: &StepTracker, out: &mut Option<Box<dyn Write>>, buffering: bool)
{
	if let Some(out) = out
	{
		if buffering
		{
			writeln!(out, "\nvvvvvvvvvv  Simulation Events  vvvvvvvvvv").unwrap();
		}
		print_events(&tracker, out);
		if buffering
		{
			writeln!(out, "\n^^^^^^^^^^  Simulation Events  ^^^^^^^^^^").unwrap();
		}
	}
}
