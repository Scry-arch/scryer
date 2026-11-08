#![cfg_attr(feature = "fail-on-warnings", deny(warnings))]

use crate::{
	cli::{
		parse_input, print_events, print_events_final, value_to_string, Cli, Target, TimeoutType,
	},
	elf::extract_elf_and_stack_base,
	events::StepTracker,
};
use scry_asm::Assemble;
use scry_sim::{
	Block, BlockedMemory, CallFrameState, ExecError, ExecState, Executor, OperandList, StackFrame,
};
use std::{
	collections::HashMap,
	io::{BufWriter, Write},
	time::Instant,
};

pub mod cli;
mod elf;
mod events;

/// Extracts the entry point, memory, address width (power of 2), and stack base
/// address of the given contents.
///
/// If necessary, will try and auto-detect the target, returning an error if
/// unsuccessful.
pub fn extract_target(
	args: &Cli,
	file_content: &[u8],
) -> Result<(usize, BlockedMemory, u8, Option<usize>), String>
{
	if matches!(args.target, Target::Scry32UnknownNoneElf | Target::Auto)
	{
		if let Ok(object::File::Elf32(elf)) = object::File::parse(file_content)
		{
			extract_elf_and_stack_base(elf, &args.stack_base)
				.map_err(|_| format!("Missing stack base label: {}", &args.stack_base))
		}
		else
		{
			Err("File does not match scry32-unknown-none-elf".into())
		}
	}
	else
	{
		Err("".into())
	}
	.or_else(|elf32_err| {
		if matches!(args.target, Target::Scry64UnknownNoneElf | Target::Auto)
		{
			if let Ok(object::File::Elf64(elf)) = object::File::parse(file_content)
			{
				extract_elf_and_stack_base(elf, &args.stack_base)
					.map_err(|_| format!("Missing stack base label: {}", &args.stack_base))
			}
			else
			{
				Err("File does not match scry64-unknown-none-elf".into())
			}
		}
		else
		{
			Err(elf32_err)
		}
	})
	.or_else(|elf64_err| {
		if matches!(args.target, Target::Assembly | Target::Auto)
		{
			// File is in textual assembly, assemble it
			str::from_utf8(file_content)
				.map_err(|_| "File not UTF-8".into())
				.and_then(|text| {
					scry_asm::Raw::assemble(std::iter::once(text))
						.map_err(|_| "File not valid assembly".into())
						.map(|program| (0, BlockedMemory::new(program.into_iter(), 0), 2, None))
				})
		}
		else
		{
			Err(elf64_err)
		}
	})
	.or_else(|assembly_err| {
		if matches!(args.target, Target::Raw)
		{
			Ok((
				0,
				BlockedMemory::new(file_content.into_iter().cloned(), 0),
				2,
				None,
			))
		}
		else
		{
			if args.target == Target::Auto
			{
				Err("Unable to determine target".into())
			}
			else
			{
				Err(assembly_err)
			}
		}
	})
}

pub fn run(args: Cli, mut stdout: impl Write, _stderr: impl Write) -> i32
{
	// Read program
	let contents = std::fs::read(&args.path).unwrap();

	// Prepare event output
	let mut event_print_out = args.print_events.clone().map(|p| -> Box<dyn Write> {
		match p
		{
			None => Box::new(std::io::stdout()),
			Some(path) => Box::new(BufWriter::new(std::fs::File::create(path).unwrap())),
		}
	});

	// Default stack base
	let mut stack_base = 1 << 20;

	// Check for absolute stack base
	if args.stack_base.starts_with("0x")
	{
		if let Ok(addr) = usize::from_str_radix(args.stack_base.trim_start_matches("0x"), 16)
		{
			stack_base = addr;
		}
	}
	else if let Ok(addr) = args.stack_base.parse::<usize>()
	{
		stack_base = addr;
	}

	let (entry, mut memory, address_space) = match extract_target(&args, contents.as_slice())
	{
		Ok((e, m, a, s)) =>
		{
			if let Some(addr) = s
			{
				stack_base = addr;
			}
			(e, m, a)
		},
		Err(err) =>
		{
			eprintln!("Invalid Target: {:?}", err);
			return 123;
		},
	};

	// Ready inputs
	let mut op_queue = HashMap::new();
	if !args.input.is_empty()
	{
		let mut ops: Vec<_> = args
			.input
			.iter()
			.map(|s| {
				let labels = [("entry", entry)].into_iter().collect();
				parse_input(s, labels)
			})
			.collect();
		op_queue.insert(0, OperandList::new(ops.remove(0), ops));
	}

	let base_stack = StackFrame {
		block: Block {
			address: stack_base,
			size: 0,
		},
		private_size: 0,
	};
	let original_state = ExecState {
		addr_space: address_space,
		address: entry,
		frame: CallFrameState {
			ret_addr: 0,
			branches: HashMap::new(),
			op_queue,
			stack: base_stack.clone(),
		},
		frame_stack: vec![CallFrameState {
			ret_addr: 0,
			branches: HashMap::new(),
			op_queue: HashMap::new(),
			stack: base_stack,
		}],
		stack_buffer: args.stack_buffer,
		more_depth: 1, // Total depth = 4
	};
	let mut events = StepTracker::new();

	// Add stack memery block
	memory.add_block((0..args.stack_buffer).map(|_| 0), stack_base);

	let mut res =
		Executor::<BlockedMemory, _>::from_state(&original_state, &mut memory, &mut events)
			.step(&mut events);

	let start_time = Instant::now();
	let mut step_count = 1;
	while res.is_ok()
	{
		let exec = res.unwrap();
		let state = exec.state();

		if state.frame_stack.len() == 0
		{
			// Done
			if let Some(returned_values) = state.frame.op_queue.get(&0)
			{
				print_events_final(&events, &mut event_print_out, args.print_events_buffered);
				if args.machine_mode
				{
					// Return the integer value of the first return operand or
					// 123 if unavailable
					return returned_values
						.iter()
						.next()
						.unwrap()
						.get_first()
						.bytes()
						.map_or(123, |b| b[0] as i32);
				}
				else
				{
					// Pretty print the returned operands
					writeln!(stdout, "----------  Returned Operands  ----------").unwrap();
					for val in returned_values.iter()
					{
						let val_str = value_to_string(val);
						write!(stdout, "{}, ", val_str).unwrap();
					}

					return 0;
				}
			}
			// Failure
			res = Err(ExecError::Err("No operands returned by simulation.".into()));
			continue;
		}

		if args.timeout > 0
			&& ((args.timeout_type == TimeoutType::Instructions
				&& step_count == args.timeout) // We assume one instruction per step
				| (args.timeout_type == TimeoutType::Seconds
					&& start_time.elapsed().as_secs() > args.timeout as u64))
		{
			print_events_final(&events, &mut event_print_out, args.print_events_buffered);
			if !args.machine_mode
			{
				writeln!(stdout, "----------  Timeout  ----------").unwrap();
			}
			return 123;
		}

		if let Some(out) = &mut event_print_out
		{
			if !args.print_events_buffered
			{
				print_events(&events, out);
				events.clear_events()
			}
		}

		events.new_step();
		res = exec.step(&mut events);
		step_count += 1;
	}
	// Implicit failure
	match res
	{
		Err(err) =>
		{
			print_events_final(&events, &mut event_print_out, args.print_events_buffered);
			if !args.machine_mode
			{
				writeln!(stdout, "----------  Error  ----------").unwrap();
				writeln!(stdout, "{:?}", err).unwrap();
			}
		},
		Ok(_) => unreachable!(),
	}
	123
}
