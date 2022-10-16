use crate::TEMPORARY_DIR;
use assert_cmd::Command;
use duplicate::duplicate_item;
use predicates::prelude::predicate;
use scry_asm::Assemble;
use scry_sim::{Metric::*, MetricReporter, TrackReport};
use std::{io::Write, time::Duration};

/// Tests that the given assembly program can be simulated with the given inputs
/// to produce the given output.
///
/// The tests are run directly on the binary.
/// Each program is output to a file which is then given to the binary.
///
/// We test each program and input/output in the following configurations:
///
/// 1. As textual assembly
/// 1. As binary assembly (using `-b` flag)
///
/// All programs should terminate within 5 seconds (otherwise they are timed
/// out).
fn test_program<const INS: usize>(
	program: &str,
	inputs: [&str; INS],
	expected_mahine_result: u8,
	expected_result: &str,
	test_binary: bool,
	machine_mode: bool,
	expected_metrics: TrackReport,
) -> Result<(), Box<dyn std::error::Error>>
{
	let file_content = if test_binary
	{
		scry_asm::Raw::assemble(std::iter::once(program)).unwrap()
	}
	else
	{
		program.as_bytes().iter().cloned().collect()
	};

	// Output program to a file
	std::fs::create_dir_all(TEMPORARY_DIR)?;
	let file = tempfile::Builder::new().tempfile_in(TEMPORARY_DIR)?;
	file.as_file().write_all(file_content.as_slice())?;

	// Run on the file with the given inputs
	let mut cmd = Command::cargo_bin("scryer")?;
	cmd.arg(file.path());
	if machine_mode
	{
		cmd.arg("--machine-mode");
	}
	if test_binary
	{
		cmd.arg("--binary");
	}
	for input in inputs
	{
		cmd.arg("-i=".to_owned() + input);
	}

	// Check Results
	let assert = cmd.timeout(Duration::new(5, 0)).assert();
	let assert = if machine_mode
	{
		assert
			.code(predicate::eq(expected_mahine_result as i32))
			.stdout(predicates::str::is_empty())
			.stderr(predicates::str::is_empty())
	}
	else
	{
		assert.success().stdout(
			predicate::str::is_match("Returned Operands(.)*?\n".to_owned() + expected_result)
				.unwrap(),
		)
	};

	// Check Metrics
	if !machine_mode
	{
		let mut regex = r"Simulation Metrics(.)*?\n(.|\n)*?".to_owned();

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
			let metric_val = expected_metrics.get_stat(metric);
			if metric_val != 0
			{
				regex.push_str(
					format!(r"{:?}(\s)*?:(\s)*?{}\n(.|\n)*?", metric, metric_val).as_str(),
				);
			}
		}

		assert.stdout(predicate::str::is_match(regex).unwrap());
	}

	// Success
	Ok(())
}

/// Tests a given program on a set of inputs and outputs.
///
/// First an identifier must be given that is used as a prefix for the names of
/// each test case (which is otherwise made up of the input strings).
///
/// Then, inside `[]` a set of input-output pairs are given.
/// Each pair starts with an array of inputs as strings, followed by `->`
/// followed by the expected output value.
///
/// Lastly, the program itself is given in assembly text format.
macro_rules! test_program {
	(
		$name:ident
		[
			$(
				[$($inputs:literal),+] -> [$expected_machine_out:literal, $expected_out:literal] :
				[ $($metric:ident : $value:expr)+ ]
			)+
		]
		$($program:tt)*
	)=> {
		paste::paste!{
			#[allow(non_upper_case_globals)]
			const [< PROGRAM_ $name >]: &'static str = stringify!($($program)*);
			test_program! {
				@impl
				@cases [
					$(
						[< $name $(_ $inputs)+ >]
									[
							[$($inputs),+] -> [$expected_machine_out, $expected_out]
							: [$($metric : $value)+]
						]
					)+
				]
				@program [< PROGRAM_ $name >]
			}
		}
	};

	(
		/// Expand each case individually
		@impl
		@cases [
			$name:ident
			[
				[$($inputs:literal),+] -> [$expected_machine_out:literal, $expected_out:literal] :
				[ $($metric:ident : $value:expr)+ ]
			]
			$($rest:tt)*
		]
		@program $program:ident
	) => {
		paste::paste!{
			#[test]
			#[allow(non_snake_case)]
			fn [< $name _assembly>]() -> Result<(), Box<dyn std::error::Error>>{
				test_program($program, [$($inputs,)+], $expected_machine_out, $expected_out,
					false, false, [$(($metric, $value),)+].into())
			}
			#[test]
			#[allow(non_snake_case)]
			fn [< $name _binary>]() -> Result<(), Box<dyn std::error::Error>>{
				test_program($program, [$($inputs,)+], $expected_machine_out, $expected_out,
					true, false, [$(($metric, $value),)+].into())
			}
			#[test]
			#[allow(non_snake_case)]
			fn [< $name _assembly_machine>]() -> Result<(), Box<dyn std::error::Error>>{
				test_program($program, [$($inputs,)+], $expected_machine_out, $expected_out,
					false, true, [$(($metric, $value),)+].into())
			}
			#[test]
			#[allow(non_snake_case)]
			fn [< $name _binary_machine>]() -> Result<(), Box<dyn std::error::Error>>{
				test_program($program, [$($inputs,)+], $expected_machine_out, $expected_out,
					true, true, [$(($metric, $value),)+].into())
			}
		}
		test_program!{
			@impl
			@cases [$($rest)*]
			@program $program
		}
	};

	(
		/// No more cases
		@impl
		@cases []
		@program $program:tt
	) => {};
}

#[duplicate_item(
	shared_metrics(operand_bytes) [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 1
		ConsumedBytes		: operand_bytes
		QueuedValues		: 1
		QueuedValueBytes	: operand_bytes
		InstructionReads	: 2
	];
)]
test_program! {
	increment [
		["0u0"] 	-> [1, "1u0"]	: [ shared_metrics([1]) ]
		["1i1"] 	-> [2, "2i1"]	: [ shared_metrics([2]) ]
		["2u2"] 	-> [3, "3u2"]	: [ shared_metrics([4]) ]
		["255u3"] 	-> [0, "256u3"]	: [ shared_metrics([8]) ]
	]
				inc =>ret_at
				ret ret_at
	ret_at:
}

#[duplicate_item(
	shared_metrics(operand_bytes) [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 3
		ConsumedBytes		: operand_bytes*3
		QueuedValues		: 2
		QueuedValueBytes	: operand_bytes*2
		InstructionReads	: 3
	];
)]
test_program! {
	add_increment [
		["0u3", "0u3"]		-> [1, "1u3"]		: [ shared_metrics([8]) ]
		["0u2", "123u2"]	-> [124, "124u2"]	: [ shared_metrics([4]) ]
		["-1i1", "4i1"]		-> [4, "4i1"]		: [ shared_metrics([2]) ]
		["2i0", "-22i0"]	-> [237, "-19i0"]	: [ shared_metrics([1]) ]
	]
				add =>0
				inc =>ret_at
				ret ret_at
	ret_at:
}

#[duplicate_item(
	shared_metrics [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 3
		ConsumedBytes		: 3
		QueuedValues		: 3
		QueuedValueBytes	: 3
		InstructionReads	: 4
	];
)]
#[duplicate_item(
	test_name constant metrics;
	[add_const_unsigned] [ 12u0] [
		["0u0"]		-> [13, "13u0"]	: [ shared_metrics ]
		["25u0"]	-> [38, "38u0"]	: [ shared_metrics ]
	];
	[add_const_signed] [ 54i0] [
		["-54i0"]	-> [1, "1i0"]	: [ shared_metrics ]
		["43i0"]	-> [98, "98i0"]	: [ shared_metrics ]
	];
)]
test_program! {
	test_name [ metrics	]
				inc =>add_to
				ret ret_at
	add_to:		const constant
				add =>ret_at
	ret_at:
}

#[duplicate_item(
	shared_metrics(operand_bytes) [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 4
		ConsumedBytes		: operand_bytes*4
		QueuedValues		: 4
		QueuedValueBytes	: operand_bytes*4
		ReorderedOperands	: 1
		InstructionReads	: 4
	];
)]
test_program! {
	triple_using_duplicate [
		["0u3"]		-> [0, "0u3"]		: [ shared_metrics([8]) ]
		["12u2"]	-> [36, "36u2"]	: [ shared_metrics([4]) ]
		["-5i1"]	-> [241, "-15i1"]	: [ shared_metrics([2]) ]
		["14i0"]	-> [42, "42i0"]		: [ shared_metrics([1]) ]
	]
				dup =>add1, =>add2, =>
	add1:		add =>add2
				ret ret_at
	add2:		add =>0
	ret_at:
}

#[duplicate_item(
	shared_metrics(operand_bytes) [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 4
		ConsumedBytes		: operand_bytes*4
		QueuedValues		: 2
		QueuedValueBytes	: operand_bytes*2
		ReorderedOperands	: 3
		InstructionReads	: 4
	];
)]
test_program! {
	three_way_add_using_echo [
		["0u2","1u2","2u2"]		-> [3, "3u2"]		: [ shared_metrics([4]) ]
		["-5i2","-16i2","21i2"]	-> [0, "0i2"]		: [ shared_metrics([4]) ]
	]
				echo =>add1, =>add2, =>
	add1:		add =>add2
				ret ret_at
	add2:		add =>0
	ret_at:
}

#[duplicate_item(
	shared_metrics(operand_bytes) [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 1
		ConsumedBytes		: operand_bytes*1
		QueuedValues		: 1
		QueuedValueBytes	: operand_bytes*1
		ReorderedOperands	: 1
		InstructionReads	: 45
	];
)]
test_program! {
	long_echo_with_nops [
		["0u0"]		-> [255, "255u0"]	: [ shared_metrics([1]) ]
		["123u2"]	-> [122, "122u2"]	: [ shared_metrics([4]) ]
		["-124i1"]	-> [131, "-125i1"]	: [ shared_metrics([2]) ]
		["14i3"]	-> [13, "13i3"]		: [ shared_metrics([8]) ]
	]
				echo =>to_add
				// 40 nops ensure that only EchoLong can be used
				nop
				nop
				nop
				nop
				nop
				nop
				nop
				nop
				nop
				nop

				nop
				nop
				nop
				nop
				nop
				nop
				nop
				nop
				nop
				nop

				nop
				nop
				nop
				nop
				nop
				nop
				nop
				nop
				nop
				nop

				nop
				nop
				nop
				nop
				nop
				nop
				nop
				nop
				nop
				nop

				ret ret_at
	to_add:		dec =>ret_at
				nop
				nop
	ret_at:
}

#[duplicate_item(
	shared_metrics(operand_bytes) [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 3
		ConsumedBytes		: operand_bytes*3
		QueuedValues		: 2
		QueuedValueBytes	: operand_bytes*2
		ReorderedOperands	: 2
		InstructionReads	: 5
		IssuedBranches		: 1
		TriggeredBranches	: 1
	];
)]
test_program! {
	unconditional_jump [
		["0u2","1u2"]	-> [2, "2u2"]			: [ shared_metrics([4]) ]
		["5i3","16i3"]	-> [22, "22i3"]			: [ shared_metrics([8]) ]
		["-1i0","123i0"]	-> [123, "123i0"]	: [ shared_metrics([1]) ]
	]
				echo =>inc1, =>after_inc1=>add1
				ret ret_at
				jmp add1, after_inc1
	inc1:		inc =>after_inc1=>add1
	after_inc1:
				nop
				nop
				nop
				nop
	add1:		add =>0
	ret_at:
}

#[duplicate_item(
	shared_metrics(operand_bytes) [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 3
		ConsumedBytes		: operand_bytes*3
		QueuedValues		: 2
		QueuedValueBytes	: operand_bytes*2
		ReorderedOperands	: 2
		InstructionReads	: 6
		IssuedBranches		: 2
		TriggeredBranches	: 2
	];
)]
test_program! {
	jump_backwards [
		["0u2","1u2"]	-> [2, "2u2"]			: [ shared_metrics([4]) ]
		["5i3","16i3"]	-> [22, "22i3"]			: [ shared_metrics([8]) ]
		["-1i0","123i0"]	-> [123, "123i0"]	: [ shared_metrics([1]) ]
	]
				echo =>skip_at=>skip_to=>inc1, =>skip_at=>skip_to=>add1
				jmp skip_to, skip_at // Skip the return
	skip_at:
	jmp_to:		ret 0
	skip_to:	jmp jmp_to, jmp_at
	inc1:		inc =>add1
	add1:		add =>jmp_at=>jmp_to=>skip_to
	jmp_at:
}

#[duplicate_item(
	shared_metrics(operand_bytes, branches) [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 3
		ConsumedBytes		: operand_bytes*3
		QueuedValues		: 2
		QueuedValueBytes	: operand_bytes+1
		InstructionReads	: 4
		IssuedBranches		: branches
		TriggeredBranches	: branches
	];
)]
test_program! {
	conditional_jmp [
		["0u2","1u2"]		-> [0, "0u0"]	: [ shared_metrics([4], [0]) ]
		["5i3","16i3"]		-> [0, "0u0"]	: [ shared_metrics([8], [0]) ]
		["-1i1","-1i1"]		-> [1, "1u0"]	: [ shared_metrics([2], [1]) ]
		["4i0","4i0"]		-> [1, "1u0"]	: [ shared_metrics([1], [1]) ]
	]
	// Return 1 if they are equal, 0 otherwise
				sub Low, =>0
				jmp if_equal, 0
	if_unequal:
				ret 1
				const 0u0
	if_equal:
				ret 1
				const 1u0
}

#[duplicate_item(
	shared_metrics(extra_loops) [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 5 + (4 * extra_loops)
		ConsumedBytes		: 5 + (4 * extra_loops)
		QueuedValues		: 7 + (4 * extra_loops)
		QueuedValueBytes	: 7 + (4 * extra_loops)
		InstructionReads	: 14 + (5 * extra_loops)
		IssuedBranches		: 0 + (1 * extra_loops)
		TriggeredBranches	: 0 + (1 * extra_loops)
		ReorderedOperands	: 6 + (2 * extra_loops)
	];
)]
test_program! {
	fibonacci [
		["0u0"]		-> [0, "0u0"]	: [
			IssuedReturns		: 1
			TriggeredReturns	: 1
			ConsumedOperands	: 1
			ConsumedBytes		: 1
			QueuedValues		: 2
			QueuedValueBytes	: 2
			InstructionReads	: 4
			IssuedBranches		: 1
			TriggeredBranches	: 1
			ReorderedOperands	: 1
		]
		["1u0"]		-> [1, "1u0"]		: [ shared_metrics([0]) ]
		["2u0"]		-> [1, "1u0"]		: [ shared_metrics([1]) ]
		["3u0"]		-> [2, "2u0"]		: [ shared_metrics([2]) ]
		["4u0"]		-> [3, "3u0"]		: [ shared_metrics([3]) ]
		["5u0"]		-> [5, "5u0"]		: [ shared_metrics([4]) ]
		["6u0"]		-> [8, "8u0"]		: [ shared_metrics([5]) ]
		["7u0"]		-> [13, "13u0"]		: [ shared_metrics([6]) ]
		["8u0"]		-> [21, "21u0"]		: [ shared_metrics([7]) ]
		["9u0"]		-> [34, "34u0"]		: [ shared_metrics([8]) ]
		["10u0"]	-> [55, "55u0"]		: [ shared_metrics([9]) ]
		["11u0"]	-> [89, "89u0"]		: [ shared_metrics([10]) ]
		["12u0"]	-> [144, "144u0"]	: [ shared_metrics([11]) ]
		["13u0"]	-> [233, "233u0"]	: [ shared_metrics([12]) ]
	]
	// Takes a u0 (n)(<14), returning a u0 result equals to the nth number in the fibonacci sequence.
	entry:
							dup 	=>dec_n, =>0								// Send to next jmp, and decrementor
							jmp		early_ret, 0								// If n=0, result is 0

							const 0u0											// Initial values
							const 1u0
							echo =>values, =>add_values
	loop_start:
	dec_n: 					dec 	=>0											// decrement n and send to loop condition and next decrementor
							dup 	=>0,
									=>loop_end=>dec_n
							jmp 	loop_start, loop_end						// while n>0, repeat
	values:					dup		=>loop_end=>loop_start=>add_values, =>0		// Incoming high value. Send it immediately to add, where it works as high value.
																				// send it also to add in the next iteration, where it works as low value.
	add_values:				add 	=>loop_end=>loop_start=>values				// add high and low values and output the next high value
	loop_end:
							// At this point the low value is the result.
							// wait for it to be on the ready list
							ret final_ret_trig
							nop
							nop
							nop													// Get low value as result, throw high out
	final_ret_trig:
	early_ret: 				ret early_ret_trig									// n=0, return 0
							const 0u0
	early_ret_trig:
}

#[duplicate_item(
	shared_metrics [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 2
		ConsumedBytes		: 2
		QueuedValues		: 1
		QueuedValueBytes	: 1
		QueuedReads			: 1
		DataReads			: 1
		DataReadBytes		: 1
		InstructionReads	: 3
	];
)]
test_program! {
	load_from_array [
		["6u0"] 	-> [124, "124u0"]	: [ shared_metrics ]
		["8u0"] 	-> [125, "125u0"]	: [ shared_metrics ]
		["10u0"] 	-> [126, "126u0"]	: [ shared_metrics ]
		["12u0"] 	-> [127, "127u0"]	: [ shared_metrics ]
	]
				ld u0, =>add_one
				ret ret_at
				// Add one suc that the loaded value is consumed
	add_one:	inc =>ret_at
	ret_at:

	// We use instructions as the load data.
	// Since the low-order byte of each "const" instruction contains the immediate,
	// use it to set the value
	load_from:
				const 123u0
				const 124u0
				const 125u0
				const 126u0
}

#[duplicate_item(
	shared_metrics(addr_size) [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 11
		ConsumedBytes		: 10 + addr_size
		QueuedValues		: 10
		QueuedValueBytes	: 10
		QueuedReads			: 1
		DataReads			: 1
		DataReadBytes		: 1
		ReorderedOperands	: 4
		InstructionReads	: 18
	];
)]
test_program! {
	load_before_store [
		["4u0"] 	-> [0, "0u0"]	: [ shared_metrics([1]) ]
		["5u1"] 	-> [1, "1u0"]	: [ shared_metrics([2]) ]
		["6u2"] 	-> [2, "2u0"]	: [ shared_metrics([4]) ]
		["7u3"] 	-> [3, "3u0"]	: [ shared_metrics([8]) ]
	]
				ld u0, =>consume
				ret ret_at
	data:
				// 4 bytes of data
				nop
				nop

	init_data:
				// Initialize data array to [0,1,...]
				const 4u0
				const 0u0
				st
				const 5u0
				const 1u0
				st
				const 6u0
				const 2u0
				st
				const 7u0
				const 3u0
				st

	consume:
				inc =>0
				dec =>0
	ret_at:
}
