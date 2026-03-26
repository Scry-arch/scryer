use crate::{all_tests::create_test_elf, TEMPORARY_DIR};
use assert_cmd::cargo::cargo_bin_cmd;
use duplicate::{duplicate_item, substitute_item};
use predicates::prelude::predicate;
use scry_asm::Assemble;
use scry_sim::{Metric, Metric::*, MetricReporter, TrackReport};
use std::{io::Write, iter::once, time::Duration};

/// Program file target types to test
enum Target
{
	/// Raw binary file containing only encoded instructions
	Raw,
	/// File containing textual assembly
	Assembly,
	/// ELF32 file
	ScryUnknownNoneElf32,
	/// ELF64 file
	ScryUnknownNoneElf64,
}

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
	program: &[&'static str],
	inputs: [&str; INS],
	expected_mahine_result: u8,
	expected_result: &str,
	test_target: Target,
	machine_mode: bool,
	expected_metrics: TrackReport,
) -> Result<(), Box<dyn std::error::Error>>
{
	let assembled = scry_asm::Raw::assemble(program.iter().cloned())?;
	let file_content = match test_target
	{
		Target::Raw => assembled,
		Target::Assembly =>
		{
			program
				.iter()
				.flat_map(|snippet| snippet.as_bytes().into_iter().chain(once(&('\n' as u8))))
				.cloned()
				.collect()
		},
		Target::ScryUnknownNoneElf32 =>
		{
			let elf = create_test_elf(assembled.as_slice(), 0, false);
			let mut out = object::write::StreamingBuffer::new(Vec::new());
			elf.write(&mut out)?;
			out.into_inner()
		},
		Target::ScryUnknownNoneElf64 =>
		{
			let elf = create_test_elf(assembled.as_slice(), 0, true);
			let mut out = object::write::StreamingBuffer::new(Vec::new());
			elf.write(&mut out)?;
			out.into_inner()
		},
	};

	// Output program to a file
	std::fs::create_dir_all(TEMPORARY_DIR)?;
	let file = tempfile::Builder::new().tempfile_in(TEMPORARY_DIR)?;
	file.as_file().write_all(file_content.as_slice())?;

	// Run on the file with the given inputs
	let mut cmd = cargo_bin_cmd!("scryer");
	cmd.arg(file.path());
	// cmd.arg("--debug");
	if machine_mode
	{
		cmd.arg("--machine-mode");
	}
	cmd.arg(match test_target
	{
		Target::Raw => "--target=raw",
		Target::Assembly => "--target=assembly",
		Target::ScryUnknownNoneElf32 => "--target=scry-unknown-none-elf32",
		Target::ScryUnknownNoneElf64 => "--target=scry-unknown-none-elf64",
	});
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
			predicate::str::is_match(
				"Returned Operands(.)*?\\n".to_owned()
					+ expected_result
					// Ensure only a superfluous comma and whitespace may follow the expected
					 + "(, )?\\n",
			)
			.unwrap(),
		)
	};

	// Check Metrics
	if !machine_mode
	{
		let mut regex = r"Simulation Metrics(.)*?\n(.|\n)*?".to_owned();

		for metric in Metric::all()
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
				[ $($metric:ident : $value:expr)* ]
			)+
		]
		$($program:literal)*
	)=> {
		paste::paste!{
			#[allow(non_upper_case_globals)]
			const [< PROGRAM_ $name >]: &'static [&'static str] = &[$($program),*];
			test_program! {
				@impl
				@cases [
					$(
						[< $name $(_ $inputs)+ >]
									[
							[$($inputs),+] -> [$expected_machine_out, $expected_out]
							: [$($metric : $value)*]
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
				[ $($metric:ident : $value:expr)* ]
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
					Target::Assembly, false, [$(($metric, $value),)*].into())
			}
			#[test]
			#[allow(non_snake_case)]
			fn [< $name _binary>]() -> Result<(), Box<dyn std::error::Error>>{
				test_program($program, [$($inputs,)+], $expected_machine_out, $expected_out,
					Target::Raw, false, [$(($metric, $value),)*].into())
			}
			#[test]
			#[allow(non_snake_case)]
			fn [< $name _elf32>]() -> Result<(), Box<dyn std::error::Error>>{
				test_program($program, [$($inputs,)+], $expected_machine_out, $expected_out,
					Target::ScryUnknownNoneElf32, false, [$(($metric, $value),)*].into())
			}
			#[test]
			#[allow(non_snake_case)]
			fn [< $name _elf64>]() -> Result<(), Box<dyn std::error::Error>>{
				test_program($program, [$($inputs,)+], $expected_machine_out, $expected_out,
					Target::ScryUnknownNoneElf64, false, [$(($metric, $value),)*].into())
			}
			#[test]
			#[allow(non_snake_case)]
			fn [< $name _assembly_machine>]() -> Result<(), Box<dyn std::error::Error>>{
				test_program($program, [$($inputs,)+], $expected_machine_out, $expected_out,
					Target::Assembly, true, [$(($metric, $value),)*].into())
			}
			#[test]
			#[allow(non_snake_case)]
			fn [< $name _binary_machine>]() -> Result<(), Box<dyn std::error::Error>>{
				test_program($program, [$($inputs,)+], $expected_machine_out, $expected_out,
					Target::Raw, true, [$(($metric, $value),)*].into())
			}
			#[test]
			#[allow(non_snake_case)]
			fn [< $name _elf32_machine>]() -> Result<(), Box<dyn std::error::Error>>{
				test_program($program, [$($inputs,)+], $expected_machine_out, $expected_out,
					Target::ScryUnknownNoneElf32, true, [$(($metric, $value),)*].into())
			}
			#[test]
			#[allow(non_snake_case)]
			fn [< $name _elf64_machine>]() -> Result<(), Box<dyn std::error::Error>>{
				test_program($program, [$($inputs,)+], $expected_machine_out, $expected_out,
					Target::ScryUnknownNoneElf64, true, [$(($metric, $value),)*].into())
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

#[substitute_item(
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
		["0u8"] 	-> [1, "1u8"]	: [ shared_metrics([1]) ]
		["1i16"] 	-> [2, "2i16"]	: [ shared_metrics([2]) ]
		["2u32"] 	-> [3, "3u32"]	: [ shared_metrics([4]) ]
		["255u64"] 	-> [0, "256u64"]	: [ shared_metrics([8]) ]
	]
				"add Low =>ret_at"
				"ret ret_at"
	"ret_at:"
}

#[substitute_item(
	shared_metrics(operand_bytes, minus_consumed) [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 3
		ConsumedBytes		: (operand_bytes*3)-minus_consumed
		QueuedValues		: 2
		QueuedValueBytes	: operand_bytes*2
		InstructionReads	: 3
	];
)]
test_program! {
	add_increment [
		["0u64", "0u64"]		-> [1, "1u64"]		: [ shared_metrics([8],[0]) ]
		["0u32", "123u32"]		-> [124, "124u32"]	: [ shared_metrics([4],[0]) ]
		["-1i16", "4i16"]		-> [4, "4i16"]		: [ shared_metrics([2],[0]) ]
		["2i8", "-22i8"]		-> [237, "-19i8"]	: [ shared_metrics([1],[0]) ]
		["1i16", "-1i8"]		-> [1, "1i16"]		: [ shared_metrics([2],[1]) ]
		["668i16", "247u32"]	-> [148, "916u32"]	: [ shared_metrics([4],[2]) ]
		["56797u64", "57u8"]	-> [23, "56855u64"]	: [ shared_metrics([8],[7]) ]
	]
				"add =>0"
				"add Low =>ret_at"
				"ret ret_at"
	"ret_at:"
}

#[substitute_item(
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
	test_name typ constant metrics;
	[add_const_unsigned] [ "u8" ] [ "12" ] [
		["0u8"]		-> [13, "13u8"]	: [ shared_metrics ]
		["25u8"]	-> [38, "38u8"]	: [ shared_metrics ]
	];
	[add_const_signed] [ "i8" ] [ "54" ] [
	["-54i8"]	-> [1, "1i8"]	: [ shared_metrics ]
	["43i8"]	-> [98, "98i8"]	: [ shared_metrics ]
	];
	[add_const_signed_negative] [ "i8" ] [ "-2" ] [
		["-24i8"]	-> [231, "-25i8"]	: [ shared_metrics ]
		["6i8"]		-> [5, "5i8"]	: [ shared_metrics ]
	];
)]
test_program! {
	test_name [ metrics	]
				"add Low =>add_to"
				"ret ret_at"
	"add_to:"	"const" typ "," constant
				"add =>ret_at"
	"ret_at:"
}

#[substitute_item(
	shared_metrics(operand_bytes) [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 1
		ConsumedBytes		: operand_bytes
		QueuedValues		: 2
		QueuedValueBytes	: 2
		InstructionReads	: 5
		ReorderedOperands	: 3
	];
)]
test_program! {
	pick_between_2 [
		["0u8"] 	-> [123, "123i8"]	: [ shared_metrics([1]) ]
		["1i16"] 	-> [234, "234u8"]	: [ shared_metrics([2]) ]
	]
					"echo =>pick_instr"
					"const u8, 234"
					"const i8, 123"
	"pick_instr:"	"pick =>1"
					"ret 0"
}

#[substitute_item(
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
		["0u64"]		-> [0, "0u64"]		: [ shared_metrics([8]) ]
		["12u32"]	-> [36, "36u32"]	: [ shared_metrics([4]) ]
		["-5i16"]	-> [241, "-15i16"]	: [ shared_metrics([2]) ]
		["14i8"]	-> [42, "42i8"]		: [ shared_metrics([1]) ]
	]
				"dup =>add1, =>add2, =>"
	"add1:"		"add =>add2"
				"ret ret_at"
	"add2:"		"add =>0"
	"ret_at:"
}

#[substitute_item(
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
		["0u32","1u32","2u32"]		-> [3, "3u32"]		: [ shared_metrics([4]) ]
		["-5i32","-16i32","21i32"]	-> [0, "0i32"]		: [ shared_metrics([4]) ]
	]
				"echo =>add1, =>add2, =>"
	"add1:"		"add =>add2"
				"ret ret_at"
	"add2:"		"add =>0"
	"ret_at:"
}

#[substitute_item(
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
		["0u8"]		-> [255, "255u8"]	: [ shared_metrics([1]) ]
		["123u32"]	-> [122, "122u32"]	: [ shared_metrics([4]) ]
		["-124i16"]	-> [131, "-125i16"]	: [ shared_metrics([2]) ]
		["14i64"]	-> [13, "13i64"]		: [ shared_metrics([8]) ]
	]
				"echo =>to_add"
				// 40 nops ensure that only EchoLong can be used
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"

				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"

				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"

				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"
				"nop"

				"ret ret_at"
	"to_add:"	"sub Low =>ret_at"
				"nop"
				"nop"
	"ret_at:"
}

#[substitute_item(
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
		["0u32","1u32"]	-> [2, "2u32"]			: [ shared_metrics([4]) ]
		["5i64","16i64"]	-> [22, "22i64"]			: [ shared_metrics([8]) ]
		["-1i8","123i8"]	-> [123, "123i8"]	: [ shared_metrics([1]) ]
	]
				"echo =>inc1, =>after_inc1=>add1"
				"ret ret_at"
				"jmp add1, after_inc1"
	"inc1:"		"add Low =>after_inc1=>add1"
	"after_inc1:"
				"nop"
				"nop"
				"nop"
				"nop"
	"add1:"		"add =>0"
	"ret_at:"
}

#[substitute_item(
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
		["0u32","1u32"]	-> [2, "2u32"]			: [ shared_metrics([4]) ]
		["5i64","16i64"]	-> [22, "22i64"]			: [ shared_metrics([8]) ]
		["-1i8","123i8"]	-> [123, "123i8"]	: [ shared_metrics([1]) ]
	]
				"echo =>skip_at=>skip_to=>inc1, =>skip_at=>skip_to=>add1"
				"jmp skip_to, skip_at" // Skip the return
	"skip_at:"
	"jmp_to:"	"ret 0"
	"skip_to:"	"jmp jmp_to, jmp_at"
	"inc1:"		"add Low =>add1"
	"add1:"		"add =>jmp_at=>jmp_to=>skip_to"
	"jmp_at:"
}

#[substitute_item(
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
		["0u32","1u32"]		-> [0, "0u8"]	: [ shared_metrics([4], [0]) ]
		["5i64","16i64"]		-> [0, "0u8"]	: [ shared_metrics([8], [0]) ]
		["-1i16","-1i16"]		-> [1, "1u8"]	: [ shared_metrics([2], [1]) ]
		["4i8","4i8"]		-> [1, "1u8"]	: [ shared_metrics([1], [1]) ]
	]
	// Return 1 if they are equal, 0 otherwise
				"sub Low =>0"
				"jmp if_equal, 0"
	"if_unequal:"
				"ret 1"
				"const u8, 0"
	"if_equal:"
				"ret 1"
				"const u8, 1"
}

#[substitute_item(
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
		["0u8"]		-> [0, "0u8"]	: [
			IssuedReturns		: 1
			TriggeredReturns	: 1
			ConsumedOperands	: 1
			ConsumedBytes		: 1
			QueuedValues		: 2
			QueuedValueBytes	: 2
			InstructionReads	: 4
			IssuedBranches		: 1
			TriggeredBranches	: 1
		]
		["1u8"]		-> [1, "1u8"]		: [ shared_metrics([0]) ]
		["2u8"]		-> [1, "1u8"]		: [ shared_metrics([1]) ]
		["3u8"]		-> [2, "2u8"]		: [ shared_metrics([2]) ]
		["4u8"]		-> [3, "3u8"]		: [ shared_metrics([3]) ]
		["5u8"]		-> [5, "5u8"]		: [ shared_metrics([4]) ]
		["6u8"]		-> [8, "8u8"]		: [ shared_metrics([5]) ]
		["7u8"]		-> [13, "13u8"]		: [ shared_metrics([6]) ]
		["8u8"]		-> [21, "21u8"]		: [ shared_metrics([7]) ]
		["9u8"]		-> [34, "34u8"]		: [ shared_metrics([8]) ]
		["10u8"]	-> [55, "55u8"]		: [ shared_metrics([9]) ]
		["11u8"]	-> [89, "89u8"]		: [ shared_metrics([10]) ]
		["12u8"]	-> [144, "144u8"]	: [ shared_metrics([11]) ]
		["13u8"]	-> [233, "233u8"]	: [ shared_metrics([12]) ]
	]
	// Takes a u8 (n)(<14), returning a u8 result equals to the nth number in the fibonacci sequence.
	"entry:"
							"dup 	=>dec_n, =>0"								// Send to next jmp, and decrementor
							"jmp	early_ret, 0"								// If n=0, result is 0

							"const u8, 0"										// Initial values
							"const u8, 1"
							"echo =>values, =>add_values"
	"loop_start:"
	"dec_n:" 				"sub 	=>0"										// decrement n and send to loop condition and next decrementor
							"dup 	=>0,"
									"=>loop_end=>dec_n"
							"jmp 	loop_start, loop_end"						// while n>0, repeat
	"values:"				"dup		=>loop_end=>loop_start=>add_values, =>0"// Incoming high value. Send it immediately to add, where it works as high value.
																				// send it also to add in the next iteration, where it works as low value.
	"add_values:"			"add 	=>loop_end=>loop_start=>values"				// add high and low values and output the next high value
	"loop_end:"
							// At this point the low value is the result.
							// wait for it to be on the ready list
							"ret final_ret_trig"
							"nop"										// Get low value as result, throw high out
							"nop"
							"nop"
	"final_ret_trig:"
	"early_ret:" 			"ret early_ret_trig"								// n=0, return 0
							"const u8, 0"
	"early_ret_trig:"
}

#[substitute_item(
	shared_metrics [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 1
		ConsumedBytes		: 1
		QueuedValues		: 1
		QueuedValueBytes	: 1
		DataReads			: 1
		DataReadBytes		: 1
		InstructionReads	: 2
	];
)]
test_program! {
	load_from_array [
		["4u8"] 	-> [123, "123u8"]	: [ shared_metrics ]
		["5u8"] 	-> [124, "124u8"]	: [ shared_metrics ]
		["6u8"] 	-> [125, "125u8"]	: [ shared_metrics ]
		["7u8"] 	-> [126, "126u8"]	: [ shared_metrics ]
	]
				"ld u8, =>ret_at"
				"ret ret_at"
	"ret_at:"
	"load_from:"
				".bytes u8, 123"
				".bytes u8, 124"
				".bytes u8, 125"
				".bytes u8, 126"
}

#[substitute_item(
	shared_metrics [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 2
		ConsumedBytes		: 2
		QueuedValues		: 2
		QueuedValueBytes	: 2
		DataReads			: 1
		DataReadBytes		: 1
		InstructionReads	: 3
	];
)]
test_program! {
	consumed_loaded [
		["6u8"] 	-> [121, "121u8"]	: [ shared_metrics ]
		["7u8"] 	-> [122, "122u8"]	: [ shared_metrics ]
		["8u8"] 	-> [123, "123u8"]	: [ shared_metrics ]
		["9u8"] 	-> [124, "124u8"]	: [ shared_metrics ]
	]
				"ld u8, =>0"
				"add Low =>1"
				"ret 0"
	"data:"
				".bytes u8, 120"
				".bytes u8, 121"
				".bytes u8, 122"
				".bytes u8, 123"

}

#[substitute_item(
	shared_metrics(addr_size) [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		IssuedBranches		: 1
		TriggeredBranches	: 1
		ConsumedOperands	: 11
		ConsumedBytes		: 10 + addr_size
		QueuedValues		: 11
		QueuedValueBytes	: 11
		DataReads			: 1
		DataReadBytes		: 1
		ReorderedOperands	: 8
		InstructionReads	: 17
	];
)]
test_program! {
	load_before_store [
		["6u8"] 	-> [255, "255u8"]	: [ shared_metrics([1]) ]
		["7u16"] 	-> [255, "255u8"]	: [ shared_metrics([2]) ]
		["8u32"] 	-> [255, "255u8"]	: [ shared_metrics([4]) ]
		["9u64"] 	-> [255, "255u8"]	: [ shared_metrics([8]) ]
	]
				"ld u8, =>data=>init_data=>ret_at"
				"ret ret_at"
				"jmp init_data, 0"
	"data:"
				".bytes u8, 255"
				".bytes u8, 255"
	"data_3:"	".bytes u8, 255"
				".bytes u8, 255"

	"init_data:"
				// Initialize data array to [0,1,...]
				"const u8, data" // Absolute addressing
				"const u8, 0"
				"st"
				"const u8, 1" // Indexed absolute addressing
				"const u8, data"
				"const u8, 1"
				"st"
				"const i8, store_3=>data_3" // Relative addressing
				"const u8, 2"
	"store_3:"	"st"
				"const u8, 3" // Relative indexed addressing
				"const i8, store_4=>data"
				"const u8, 3"
	"store_4:"	"st"
	"ret_at:"
}

#[substitute_item(
	shared_metrics [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 5
		ConsumedBytes		: 5
		QueuedValues		: 5
		QueuedValueBytes	: 6
		DataReads			: 1
		DataReadBytes		: 2
		InstructionReads	: 6
		ReorderedOperands	: 2
	];
)]
#[duplicate_item(
	[
		name [load_from_absolute_address]
		tests [
			["0u8"] 	-> [45, "45i16"]	: [ shared_metrics ]
			["1u8"] 	-> [46, "46i16"]	: [ shared_metrics ]
			["2u8"] 	-> [47, "47i16"]	: [ shared_metrics ]
			["3u8"] 	-> [48, "48i16"]	: [ shared_metrics ]
		]
		addr_type ["u8"]
		addr_val ["22"]
	]
	[
		name [load_from_label_address]
		tests [
			["0u8"] 	-> [45, "45i16"]	: [ shared_metrics ]
			["2u8"] 	-> [47, "47i16"]	: [ shared_metrics ]
			["5u8"] 	-> [50, "50i16"]	: [ shared_metrics ]
			["7u8"] 	-> [52, "52i16"]	: [ shared_metrics ]
		]
		addr_type ["u8"]
		addr_val ["data"]
	]
	[
		name [load_from_relative_address]
		tests [
			["1i8"] 	-> [46, "46i16"]	: [ shared_metrics ]
			["3i8"] 	-> [48, "48i16"]	: [ shared_metrics ]
			["5i8"] 	-> [50, "50i16"]	: [ shared_metrics ]
			["7i8"] 	-> [52, "52i16"]	: [ shared_metrics ]
		]
		addr_type ["i8"]
		addr_val ["12"]
	]
	[
		name [load_from_relative_labels]
		tests [
			["0i8"] 	-> [45, "45i16"]	: [ shared_metrics ]
			["2i8"] 	-> [47, "47i16"]	: [ shared_metrics ]
			["4i8"] 	-> [49, "49i16"]	: [ shared_metrics ]
			["6i8"] 	-> [51, "51i16"]	: [ shared_metrics ]
		]
		addr_type ["i8"]
		addr_val ["load=>data"]
	]
)]
test_program! {
	name [ tests ]
				// Use input as index to 'data' array
				// Emulate a multiply by 2 (array element size)
				"dup =>0, =>0"
				"add =>0"
				"const " addr_type "," addr_val
				"add =>load"
				"ret 1"
	"load:"		"ld i16, =>0"
				".bytes i16, -1"
				".bytes i16, -1"
				".bytes i16, -1"
				".bytes i16, -1"
				".bytes i16, -1"
	"data:"
				".bytes i16, 45"
				".bytes i16, 46"
				".bytes i16, 47"
				".bytes i16, 48"
				".bytes i16, 49"
				".bytes i16, 50"
				".bytes i16, 51"
				".bytes i16, 52"

}

#[substitute_item(
	shared_metrics [
		IssuedReturns		: 1
		IssuedBranches		: 1
		TriggeredReturns	: 1
		TriggeredBranches	: 1
		ConsumedOperands	: 2
		ConsumedBytes		: 2
		QueuedValues		: 2
		QueuedValueBytes	: 3
		DataReads			: 1
		DataReadBytes		: 2
		InstructionReads	: 5
		ReorderedOperands	: 2
	];
)]
test_program! {
	load_from_relative_indexed [
		["0u8"] 	-> [45, "45i16"]	: [ shared_metrics ]
		["2u8"] 	-> [46, "46i16"]	: [ shared_metrics ]
		["5u8"] 	-> [47, "47i16"]	: [ shared_metrics ]
		["7u8"] 	-> [48, "48i16"]	: [ shared_metrics ]
	]
				// Jump past data
				"echo =>jmp_at=>start=>1"
				"jmp start, 0"
	"jmp_at:"
				".bytes i16, -1"
				".bytes i16, -1"
				".bytes i16, -1"
				".bytes i16, -1"
	"data:"
				".bytes i16, 45"
				".bytes i16, -1"
				".bytes i16, 46"
				".bytes i16, -1"
				".bytes i16, -1"
				".bytes i16, 47"
				".bytes i16, -1"
				".bytes i16, 48"

				".bytes i16, -1"
				".bytes i16, -1"
				".bytes i16, -1"
				".bytes i16, -1"
	"start:"	"ret 2"
				"const i8, load=>data"
	"load:"		"ld i16, =>0"
}

#[substitute_item(
	shared_metrics(len) [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 3
		ConsumedBytes		: 8 + (len*2)
		QueuedValues		: 3
		QueuedValueBytes	: 16 + len
		DataReads			: 1
		DataReadBytes		: 8
		StackReads			: 1
		StackReadBytes		: 8
		StackWrites			: 1
		StackWriteBytes		: len
		StackReserveTotal	: 1
		StackReserveTotalBytes: 64
		StackFreeTotal		: 1
		StackFreeTotalBytes	: 64
		InstructionReads	: 7
	];
)]
test_program! {
	store_and_load_from_stack [
		["0u8"] 	-> [2, "2u64"]	: [ shared_metrics([1]) ]
		["12u16"] 	-> [14, "14u64"]	: [ shared_metrics([2]) ]
		["23u32"] 	-> [25, "25u64"]	: [ shared_metrics([4]) ]
		["45u64"] 	-> [47, "47u64"]	: [ shared_metrics([8]) ]
	]
				"add Low =>store"
				"rsrv 64"
	"store:"
				"st [0]"
				"ret end"
				"ld u64 [0]"
				"add Low =>end"
				"free 64"
	"end:"
}

#[substitute_item(
	shared_metrics [
		IssuedCalls			: 1
		TriggeredCalls		: 1
		IssuedReturns		: 2
		TriggeredReturns	: 2
		ConsumedOperands	: 5
		ConsumedBytes		: 17
		QueuedValues		: 5
		QueuedValueBytes	: 17
		DataReadBytes		: 4
		StackReads			: 1
		StackReadBytes		: 4
		StackWrites			: 1
		StackWriteBytes		: 4
		StackReserveTotal	: 1
		StackReserveTotalBytes: 16
		StackReserveBase	: 1
		StackReserveBaseBytes: 8
		StackFreeTotal		: 1
		StackFreeTotalBytes	: 16
		StackFreeBase		: 0
		StackFreeBaseBytes	: 8
		InstructionReads	: 12
	];
)]
test_program! {
	pass_on_stack [
		["123u32"] 	-> [126, "126u32"]	: [ shared_metrics ]
		["75u32"] 	-> [78, "78u32"]		: [ shared_metrics ]
		["46u32"] 	-> [49, "49u32"]		: [ shared_metrics ]
	]
				"add Low =>store"
				"rsrv 16"
				"rsrv 8, Private"
	"store:"
				"st [2]"
				"const u8, func1"
				"call 0"
				"add Low =>end"
				"ret end"
				"free 16"
	"end:"
	"func1:"
				"ret func1_end"
				"ld u32 [0]"
				"add Low =>func1_end"
	"func1_end:"
}

#[substitute_item(
	shared_metrics [
		IssuedCalls			: 1
		TriggeredCalls		: 1
		IssuedReturns		: 2
		TriggeredReturns	: 2
		ConsumedOperands	: 5
		ConsumedBytes		: 9
		QueuedValues		: 5
		QueuedValueBytes	: 9
		DataReads			: 1
		DataReadBytes		: 2
		StackReads			: 1
		StackReadBytes		: 2
		StackWrites			: 1
		StackWriteBytes		: 2
		StackReserveTotal	: 0
		StackReserveTotalBytes: 2
		StackReserveBase	: 1
		StackReserveBaseBytes: 2
		StackFreeTotal		: 1
		StackFreeTotalBytes	: 2
		StackFreeBase		: 0
		StackFreeBaseBytes	: 0
		InstructionReads	: 11
	];
)]
test_program! {
	return_on_stack [
		["123u16"] 	-> [126, "126u16"]	: [ shared_metrics ]
		["75u16"] 	-> [78, "78u16"]		: [ shared_metrics ]
		["46u16"] 	-> [49, "49u16"]		: [ shared_metrics ]
	]
				"add Low =>call_args"
				"const u8, func1"
				"call 0"
	"call_args:"
				"ld u16 [0]"
				"add Low =>end"
				"ret end"
				"free 2"
	"end:"
	"func1:"
				"add Low =>store"
				"rsrv 2, Private"
	"store:"	"st [0]"
				"ret func1_end"
	"func1_end:"
}

// Tests the 'saddr' instruction returns the correct address
#[duplicate_item(
	name				size	index	result1		result2;
	[stack_addr_0_0]	["u8"]	["[0]"]	[0]			["65536u(32|64)"];
	[stack_addr_0_1]	["i8"]	["[1]"]	[1]			["65537u(32|64)"];
	[stack_addr_1_0]	["u16"]	["[0]"]	[0]			["65536u(32|64)"];
	[stack_addr_1_1]	["i16"]	["[1]"]	[2]			["65538u(32|64)"];
)]
test_program! {
	name [
		["1u8"] 	-> [result1, result2]	: [ ]
	]
				"saddr " size index
				"echo =>end"
				"ret end"
	"end:"
}

// Tests the 'saddr' instruction returns the correct address after a function
// call
#[duplicate_item(
	name							res		size	index	result1		result2;
	[stack_addr_from_call_8_0_0]	["8"]	["u8"]	["[0]"]	[8]			["65544u(32|64)"];
	[stack_addr_from_call_16_1_10]	["16"]	["i16"]	["[10]"][36]		["65572u(32|64)"];
)]
test_program! {
	name [
		["1u8"] 	-> [result1, result2]	: [ ]
	]
				"nop"
				"rsrv " res ", Private"
				"const u8, callee"
				"call 0"
				"echo =>end"
				"free " res
				"ret end"
	"end:"
	"callee:"
				"ret callee_end"
				"saddr " size index
	"callee_end:"
}

// Tests the combination of 'const' and 'grow'
#[duplicate_item(
	name						result1		result2	code;
	[immediate_0u8]				[0]			["0u8"]	[
		"const u8, 0"
	];
	[immediate_21576u16]		[72]			["21576u16"]	[
		"const u16, 84"
		"grow 72"
	];
	[immediate_6309412i32]		[36]			["6309412i32"]	[
		"const i32, 96"
		"grow 70"
		"grow 36"
	];
	[immediate_2613597195u32]	[11]			["2613597195u32"]	[
		"const u32, 155"
		"grow 200"
		"grow 84"
		"grow 11"
	];
	[immediate_211732052817i64]	[81]			["211732052817i64"]	[
		"const i64, 49"
		"grow 76"
		"grow 54"
		"grow 187"
		"grow 81"
	];
)]
test_program! {
	name [
		["0u8"] 	-> [result1, result2]	: [ ]
	]
				"ret end"
				code
	"end:"
}

#[substitute_item(
	shared_metrics(len) [
		IssuedBranches		: len-1
	    IssuedReturns		: 1
	    TriggeredBranches	: len-1
	    TriggeredReturns	: 1
	    ConsumedOperands	: len*7
	    ConsumedBytes		: len*7
	    QueuedValues		: 1+(len*8)
	    QueuedValueBytes	: 1+(len*8)
	    ReorderedOperands	: 3+(len*7)
	    InstructionReads	: 4+(len*10)
	    DataReads			: len
	    DataReadBytes		: len
	];
)]
test_program! {
	find_max [
		["28u8", "1u8"] -> [0, "0u8"]		: [ shared_metrics([1]) ]
		["28u8", "2u8"] -> [1, "1u8"]		: [ shared_metrics([2]) ]
		["30u8", "4u8"] -> [4, "4u8"]		: [ shared_metrics([4]) ]
		["35u8", "3u8"]	-> [99, "99u8"]		: [ shared_metrics([3]) ]
		["34u8", "7u8"]	-> [207, "207u8"]	: [ shared_metrics([7]) ]
	]
				// Takes as input (1) the address of an u8-array, (2) its length
				// returns the highest value in the array. (Array needn't be sorted)
	"start:"
					"echo =>dup_addr, =>dec_size"
					"const u8, 0"					// Initial max
					"echo =>dup_max"
	"loop_start:"
	"dec_size:"		"sub Low =>dup_size"
	"dup_max:"		"dup =>compare, =>pre_pick"
	"dup_size:"		"dup =>load_next, =>loop_end=>loop_start=>dec_size, =>"
	"loop_cond:"	"jmp loop_start, loop_end"
	"dup_addr:"		"dup =>load_next, =>loop_end=>loop_start=>dup_addr"
	"load_next:"	"ld u8, =>0"
					"dup =>compare, =>pre_pick"
	"compare:"		"lt =>pick_max"				// check old max less than new
	"pre_pick:"		"echo =>0"					// Pick either previous max (1) or new value (2)
												// based on comparison (0)
	"pick_max:"		"pick =>loop_end=>loop_start=>dup_max"
	"loop_end:"		"ret return"		// get final max for return by
	"return:"
	// Address: 28
	"data_1:"	".bytes u8, 0"
				".bytes u8, 1"
	// Address: 30
	"data_2:"	".bytes u8, 4"
				".bytes u8, 1"
				".bytes u8, 0"
				".bytes u8, 2"
	// Address: 34
	"data_3:"	".bytes u8, 103"
				".bytes u8, 99"
				".bytes u8, 0"
				".bytes u8, 4"
				".bytes u8, 207"
				".bytes u8, 168"
				".bytes u8, 104"
}

test_program! {
	memcpy [
		["255u8", "255u8", "0u8"]	->	[0, "0i8, 0i8, 0i8, 0i8"] : []
		["75u8", "76u8", "1u8"]		->	[7, "7i8, 0i8, 0i8, 0i8"] : []
		["74u8", "77u8", "2u8"]		->	[0, "0i8, 5i8, 7i8, 0i8"] : []
		["73u8", "77u8", "3u8"]		->	[0, "0i8, 3i8, 5i8, 7i8"] : []
		["72u8", "76u8", "4u8"]		->	[2, "2i8, 3i8, 5i8, 7i8"] : []
	]
						// Takes source address, destination address and length.
						// Copies the source array with given length to the destination.
	"start:"
						"echo =>dup_source, =>dup_sink, =>"
						"dup =>check_zero, =>dec_count"
	"check_zero:"		"jmp loop_end, 1"		//if count is zero, skip loop after return.
						"ret return"
	"dup_source:"		"dup =>load_next, =>inc_source"

	"loop_start:"
	"load_next:"		"ld u8, =>store_copy"
	"dec_count:"		"sub Low =>0"
						"dup =>loop_cond, =>loop_end=>loop_start=>dec_count"
	"loop_cond:"		"jmp loop_start, loop_end"
	"dup_sink:"			"dup =>store_copy, =>inc_sink"
	"inc_source:"		"add Low =>0"
						"dup =>loop_end=>loop_start=>load_next,"
							"=>loop_end=>loop_start=>inc_source"
	"inc_sink:"			"add Low =>loop_end=>loop_start=>dup_sink"
	"store_copy:"		"st"
	"loop_end:"
						"nop"				// Throw out values surviving the loop exit
						"nop"
						"nop"
						"nop"
						"nop"
						"nop"
						"nop"
						"nop"
						"nop"
						"nop"

						"const u8, dst1"
						"ld i8, =>return"
						"const u8, dst2"
						"ld i8, =>return"
						"const u8, dst3"
						"ld i8, =>return"
						"const u8, dst4"
						"ld i8, =>0"
	"return:"
						"nop"
						"nop"
						"nop"
						"nop"
	// Address: 72
	"src:"
						".bytes i8, 2"
						".bytes i8, 3"
						".bytes i8, 5"
						".bytes i8, 7"

	"dst1:"				".bytes i8, 0"
	"dst2:"				".bytes i8, 0"
	"dst3:"				".bytes i8, 0"
	"dst4:"				".bytes i8, 0"
}

test_program! {
	strcpy [
		["63u8", "60u8"]	->	[255, "255u8, 223u8, 0u8, 255u8"] : []
		["63u8", "59u8"]	->	[255, "255u8, 211u8, 223u8, 0u8"] : []
		["62u8", "58u8"]	->	[199, "199u8, 211u8, 223u8, 0u8"] : []
	]
					// Takes destination address and source address.
					// Copies the string from source to destination including final null character
	"start:"
						"echo =>dup_dst,  =>dup_src"
						"ret return_at"

	"loop_start:"
	"dup_src:"			"dup =>load, =>inc_src"
	"load:"				"ld u8, =>0"
						"dup =>loop_cond, =>store"
	"loop_cond:"		"jmp loop_start, loop_end"
	"dup_dst:"			"dup =>store, =>0"
						"add Low =>loop_end=>loop_start=>dup_dst"
	"inc_src:"			"add Low =>loop_end=>loop_start=>dup_src"
	"store:"			"st"
	"loop_end:"
						"nop"// Throw out values surviving the loop exit
						"nop"
						"nop"
						"nop"
						"nop"
						"nop"
						"nop"
						"nop"

						"const u8, dst1"
						"ld u8, =>return_at"
						"const u8, dst2"
						"ld u8, =>return_at"
						"const u8, dst3"
						"ld u8, =>return_at"
						"const u8, dst4"
						"ld u8, =>0"
	"return_at:"
						"nop"
						"nop"
						"nop"
	// Address: 58
	"src:"
						".bytes u8, 199"
						".bytes u8, 211"
						".bytes u8, 223"
						".bytes u8, 0"

	"dst1:"				".bytes u8, 255"
	"dst2:"				".bytes u8, 255"
	"dst3:"				".bytes u8, 255"
	"dst4:"				".bytes u8, 255"
}

#[substitute_item(
	shared_metrics [
		IssuedBranches		: 0
		IssuedReturns		: 2
		IssuedCalls			: 1
		TriggeredBranches	: 0
		TriggeredReturns	: 2
		TriggeredCalls		: 1
		ConsumedOperands	: 3
		ConsumedBytes		: 3
		QueuedValues		: 3
		QueuedValueBytes	: 3
		ReorderedOperands	: 3
		InstructionReads	: 8
		DataReads			: 0
		DataReadBytes		: 0
	];
)]
test_program! {
	simple_call [
		["0u8"] 	-> [3, "3u8"]		: [ shared_metrics ]
		["1u8"] 	-> [4, "4u8"]		: [ shared_metrics ]
		["2u8"] 	-> [5, "5u8"]		: [ shared_metrics ]
		["244u8"]	-> [247, "247u8"]	: [ shared_metrics ]
	]

	"entry:"
					"echo =>call_inputs"
					"const u8, addr_three"
					"call 0"
	"call_inputs:"
					"echo =>return_at"
					"ret return_at"
	"return_at:"

	"addr_three:"
					"const u8, 3"
					"add =>1"
					"ret 0"
}

#[substitute_item(
	shared_metrics [
		IssuedBranches		: 0
		IssuedReturns		: 2
		IssuedCalls			: 1
		TriggeredBranches	: 0
		TriggeredReturns	: 2
		TriggeredCalls		: 1
		ConsumedOperands	: 3
		ConsumedBytes		: 3
		QueuedValues		: 3
		QueuedValueBytes	: 3
		ReorderedOperands	: 1
		InstructionReads	: 7
		DataReads			: 0
		DataReadBytes		: 0
	];
)]
test_program! {
	queue_past_call [
		["0u8"] 	-> [3, "3u8"]		: [ shared_metrics ]
		["1u8"] 	-> [4, "4u8"]		: [ shared_metrics ]
		["2u8"] 	-> [5, "5u8"]		: [ shared_metrics ]
		["244u8"]	-> [247, "247u8"]	: [ shared_metrics ]
	]

	"entry:"
					"echo =>|=>add_result"
					"const u8, fn_return_three"
					"call 0"
	"call_inputs:"
	"add_result:"	"add =>return_at"
					"ret return_at"
	"return_at:"

	"fn_return_three:"
					"ret return_at2"
					"const u8, 3"
	"return_at2:"
}

test_program! {
	cmp_i8 [
		["0u8", "0u8"] 	-> [0, "0i8"]		: []
		["0u8", "1u8"] 	-> [142, "-114i8"]	: []
		["1u8", "0u8"] 	-> [114, "114i8"]	: []
		["0u8", "2u8"] 	-> [128, "-128i8"]	: []
	]

	"entry:"
								"echo =>entry_add1, =>entry_add2"
								// Convert indices to pointers into the array
								"const u8, data_i8"
	"entry_add1:"				"add =>fn_cmp_i8_args"
								"const u8, data_i8"
	"entry_add2:"				"add =>fn_cmp_i8_args"

								// Call
								"const u8, fn_cmp_i8"
								"call 0"
	"fn_cmp_i8_args:"
								// Return result
								"echo =>entry_ret"
								"ret 0"
	"entry_ret:"

	// bsearch comparison of i8 pointers
	"fn_cmp_i8:"
								"echo =>fn_cmp_i8_ld1, =>fn_cmp_i8_ld2"
								"ret fn_cmp_i8_ret"
	"fn_cmp_i8_ld1:"			"ld i8, =>fn_cmp_i8_sub"
	"fn_cmp_i8_ld2:"			"ld i8, =>0"
	"fn_cmp_i8_sub:"			"sub =>0"
	"fn_cmp_i8_ret:"

	"data_i8:"
								".bytes i8, -124"
								".bytes i8, -10"
								".bytes i8, 10"

}

test_program! {
	bsearch [
		// Empty array
		["2i8", "0u8", "1u8"] 	-> [0, "0u8"]		: []
		["2i16", "0u8", "2u8"] 	-> [0, "0u8"]		: []
		// 1 element array
		["-1i8", "1u8", "1u8"] 	-> [0, "0u8"]		: []
		["0i8", "1u8", "1u8"] 	-> [4, "4u8"]		: []
		["7i8", "1u8", "1u8"] 	-> [0, "0u8"]		: []
		["-1i8", "1u8", "2u8"] 	-> [0, "0u8"]		: []
		["0i8", "1u8", "2u8"] 	-> [8, "8u8"]		: []
		["7i8", "1u8", "2u8"] 	-> [0, "0u8"]		: []
		// 2 element array
		["-1i8", "2u8", "1u8"] 	-> [0, "0u8"]		: []
		["0i8", "2u8", "1u8"] 	-> [4, "4u8"]		: []
		["2i8", "2u8", "1u8"] 	-> [5, "5u8"]		: []
		["7i8", "2u8", "1u8"] 	-> [0, "0u8"]		: []
		["-1i8", "2u8", "2u8"] 	-> [0, "0u8"]		: []
		["0i8", "2u8", "2u8"] 	-> [8, "8u8"]		: []
		["2i8", "2u8", "2u8"] 	-> [10, "10u8"]		: []
		["7i8", "2u8", "2u8"] 	-> [0, "0u8"]		: []
		// 3 element array
		["-1i8", "3u8", "1u8"] 	-> [0, "0u8"]		: []
		["0i8", "3u8", "1u8"] 	-> [4, "4u8"]		: []
		["2i8", "3u8", "1u8"] 	-> [5, "5u8"]		: []
		["4i8", "3u8", "1u8"] 	-> [6, "6u8"]		: []
		["7i8", "3u8", "1u8"] 	-> [0, "0u8"]		: []
		["-1i8", "3u8", "2u8"] 	-> [0, "0u8"]		: []
		["0i8", "3u8", "2u8"] 	-> [8, "8u8"]		: []
		["2i8", "3u8", "2u8"] 	-> [10, "10u8"]		: []
		["4i8", "3u8", "2u8"] 	-> [12, "12u8"]		: []
		["7i8", "3u8", "2u8"] 	-> [0, "0u8"]		: []
		// 4 element array
		["-1i8", "4u8", "1u8"] 	-> [0, "0u8"]		: []
		["0i8", "4u8", "1u8"] 	-> [4, "4u8"]		: []
		["2i8", "4u8", "1u8"] 	-> [5, "5u8"]		: []
		["4i8", "4u8", "1u8"] 	-> [6, "6u8"]		: []
		["6i8", "4u8", "1u8"] 	-> [7, "7u8"]		: []
		["7i8", "4u8", "1u8"] 	-> [0, "0u8"]		: []
		["-1i8", "4u8", "2u8"] 	-> [0, "0u8"]		: []
		["0i8", "4u8", "2u8"] 	-> [8, "8u8"]		: []
		["2i8", "4u8", "2u8"] 	-> [10, "10u8"]		: []
		["4i8", "4u8", "2u8"] 	-> [12, "12u8"]		: []
		["6i8", "4u8", "2u8"] 	-> [14, "14u8"]		: []
		["7i8", "4u8", "2u8"] 	-> [0, "0u8"]		: []
		// 5 element array
		["-1i8", "5u8", "2u8"] 	-> [0, "0u8"]		: []
		["0i8", "5u8", "2u8"] 	-> [8, "8u8"]		: []
		["1i8", "5u8", "2u8"] 	-> [0, "0u8"]		: []
		["2i8", "5u8", "2u8"] 	-> [10, "10u8"]		: []
		["3i8", "5u8", "2u8"] 	-> [0, "0u8"]		: []
		["4i8", "5u8", "2u8"] 	-> [12, "12u8"]		: []
		["5i8", "5u8", "2u8"] 	-> [0, "0u8"]		: []
		["6i8", "5u8", "2u8"] 	-> [14, "14u8"]		: []
		["7i8", "5u8", "2u8"] 	-> [0, "0u8"]		: []
		["8i8", "5u8", "2u8"] 	-> [16, "16u8"]		: []
		["9i8", "5u8", "2u8"] 	-> [0, "0u8"]		: []
	]
								"echo =>1"
								"jmp entry, 0"
	/////////////////////////////////////////////////////////////////////////////////////////////
	// Array for i8 data
	"data_i8:" // Addr: 4
								".bytes i8, 0"
								".bytes i8, 2"
								".bytes i8, 4"
								".bytes i8, 6"
	/////////////////////////////////////////////////////////////////////////////////////////////
	// Array for i166 data
	"data_i166:" // Addr: 8
								".bytes i16, 0"
								".bytes i16, 2"
								".bytes i16, 4"
								".bytes i16, 6"
								".bytes i16, 8"
	/////////////////////////////////////////////////////////////////////////////////////////////
	// Addresses of comparison functions
	"cmp_fns:"// Addr: 18
								".bytes u8, fn_cmp_i8"
								".bytes u8, fn_cmp_i166"
	/////////////////////////////////////////////////////////////////////////////////////////////
	// Addresses of arrays
	"data_addr:" // Addr: 20
								".bytes u8, data_i8"
								".bytes u8, data_i166"
	/////////////////////////////////////////////////////////////////////////////////////////////
	// Store position for key to allow for getting its address, which is passed to bsearch
	"key_store:" // Addr: 22
								".bytes u16, 0"
	/////////////////////////////////////////////////////////////////////////////////////////////
	// bsearch comparison of i8 pointers
	"fn_cmp_i8:" // Addr: 24
								"echo =>fn_cmp_i8_ld1, =>fn_cmp_i8_ld2"
								"ret fn_cmp_i8_ret"
	"fn_cmp_i8_ld1:"			"ld i8, =>fn_cmp_i8_sub"
	"fn_cmp_i8_ld2:"			"ld i8, =>0"
	"fn_cmp_i8_sub:"			"sub =>0"
	"fn_cmp_i8_ret:"
	/////////////////////////////////////////////////////////////////////////////////////////////
	// bsearch comparison of i166 pointers
	"fn_cmp_i166:" // Addr: 34
								"echo =>fn_cmp_i166_ld1, =>fn_cmp_i166_ld2"
								"ret fn_cmp_i166_ret"
	"fn_cmp_i166_ld1:"			"ld i16, =>fn_cmp_i166_sub"
	"fn_cmp_i166_ld2:"			"ld i16, =>0"
	"fn_cmp_i166_sub:"			"sub =>0"
	"fn_cmp_i166_ret:"
	/////////////////////////////////////////////////////////////////////////////////////////////
	// Test setup
	"entry:" // Addr: 44
								"echo =>entry_store_key, =>entry_after_base, =>"
								"dup =>echo_size, =>sub_size"
								// First store the key, so that we can get its address
								"const u8, key_store"
	"entry_store_key:"			"st"
								// Choose comparison function and array based on size
	"sub_size:"					"sub Low =>0"
								"dup =>calc_cmp_fn, =>calc_data_addr"
								// Choose data array
								"const u8, data_addr"
	"calc_data_addr:"			"add =>0"
								"ld u8, =>entry_after_key" // Pass base address, nitems, and size to bsearch
	"entry_after_base:"			"echo =>entry_after_key"
	"echo_size:"				"echo =>entry_after_key"
								// Choose comparison function and put its address on the stack
								"const u8, cmp_fns"
	"calc_cmp_fn:"				"add =>0"
								"ld u8, =>entry_store_cmp_fn"
								"rsrv 2"
	"entry_store_cmp_fn:"		"st [0]"
								// Call bsearch
								"const u8, fn_bsearch"
								"call call_args"
														//cmp_fn (stack)
														// size
														// nitems
														// base
	"entry_after_key:"			"const u8, key_store"	// key
	"call_args:"
								"echo =>entry_ret"
								"ret entry_ret"
	"entry_ret:"
								".bytes u32, 0"
	/////////////////////////////////////////////////////////////////////////////////////////////
	// bsearch: 90

	"fn_bsearch_equal:"					// Pivot is what we are looking for, return its addr
										"ret fn_bsearch_equal_end"
	"fn_bsearch_equal_cap:"				"echo =>fn_bsearch_equal_pick" // capture size + pivot
										"nop"	// ignore in-flight operands
										"nop"
	"fn_bsearch_equal_pick:"			"const u8, 1"					// substitute for immediate pick
										"pick =>fn_bsearch_equal_end"
										// Return pivot address
	"fn_bsearch_equal_end:"

	"fn_bsearch:"
										"echo =>fn_bsearch_dup_key, =>fn_bsearch_dup_base, =>"
										"echo =>fn_bsearch_dup_nr, =>fn_bsearch_dup_size"

	"fn_bsearch_dup_nr:"				"dup =>fn_bsearch_dup_nr2, =>fn_bsearch_check_zero"
										// If elements == 0, return null
	"fn_bsearch_check_zero:"			"jmp fn_bsearch_null, fn_bsearch_loop_start"
										"free 2, Private" // free stack so nothing is returned

	"fn_bsearch_loop_start:"
	"fn_bsearch_dup_nr2:"				"dup =>fn_bsearch_pivot_half_elements, =>|=>fn_bsearch_dec_nr, =>"

										// Check iteration
	"fn_bsearch_check_loop:"			"jmp fn_bsearch_loop_start, fn_bsearch_loop_end"

										// Issue call to comparison function
										// these instruction are here to match with fn_bsearch_null
										// So no operands from the loop are returned
										"ld u8 [0]"
										"call fn_bsearch_cmp_call_args"


	"fn_bsearch_dup_key:"				"dup =>fn_bsearch_cmp_call_args, =>|=>fn_bsearch_loop_end=>fn_bsearch_loop_start=>fn_bsearch_dup_key"
	"fn_bsearch_dup_base:"				"dup =>fn_bsearch_pivot_add, =>|=>fn_bsearch_cap_base"

										// Calculate the pivot (element to compare)
	"fn_bsearch_pivot_half_elements:"	"shr Low =>fn_bsearch_pivot_scale"
	"fn_bsearch_dup_size:"				"dup =>|=>fn_bsearch_calc_right_base, =>|=>fn_bsearch_loop_end=>fn_bsearch_loop_start=>fn_bsearch_dup_size, =>"
	"fn_bsearch_pivot_scale:"			"mul Low =>fn_bsearch_pivot_add"
	"fn_bsearch_pivot_add:"				"add Low =>fn_bsearch_pivot_dup"
	"fn_bsearch_pivot_dup:"				"dup =>fn_bsearch_cmp_call_args,\
											 =>|=>(\
												 fn_bsearch_calc_right_base, \
												 fn_bsearch_check_jmp_loc=>fn_bsearch_equal=>fn_bsearch_equal_cap\
											)"

										// Trigger call
	"fn_bsearch_cmp_call_args:"
										"dup =>fn_bsearch_check_equal, =>fn_bsearch_check_positive"

										// if 0, return pivot
	"fn_bsearch_check_equal:"			"eq =>0"
										"jmp fn_bsearch_equal, fn_bsearch_check_jmp_loc"
	"fn_bsearch_check_positive:"		"gt =>fn_bsearch_choose_base"

										// decrement nr, then halve
	"fn_bsearch_dec_nr:"				"sub =>fn_bsearch_halve_nr"
	"fn_bsearch_halve_nr:"				"shr Low =>fn_bsearch_loop_end=>fn_bsearch_loop_start=>fn_bsearch_dup_nr2"

	"fn_bsearch_check_jmp_loc:"

										// Calculate move right base
	"fn_bsearch_cap_base:"				"echo =>fn_bsearch_choose_base"
	"fn_bsearch_calc_right_base:"		"add Low =>fn_bsearch_choose_base" // pivot + size
	"fn_bsearch_choose_base:"			"pick =>fn_bsearch_loop_end=>fn_bsearch_loop_start=>fn_bsearch_dup_base"

	"fn_bsearch_loop_end:"

	"fn_bsearch_null:"					// nothing found
										"ret fn_bsearch_null_end"
										// Return NULL
										"const u8, 0"
	"fn_bsearch_null_end:"
	"fn_bsearch_end:"

}

test_program! {
	hexval [
		["0u8"] 	-> [0, "0u8"]		: [  ]
		["47u8"] 	-> [0, "0u8"]		: [  ]

		["48u8"] 	-> [0, "0u8"]		: [  ] // '0' - '9'
		["49u8"] 	-> [1, "1u8"]		: [  ]
		["50u8"] 	-> [2, "2u8"]		: [  ]
		["51u8"] 	-> [3, "3u8"]		: [  ]
		["52u8"] 	-> [4, "4u8"]		: [  ]
		["53u8"] 	-> [5, "5u8"]		: [  ]
		["54u8"] 	-> [6, "6u8"]		: [  ]
		["55u8"] 	-> [7, "7u8"]		: [  ]
		["56u8"] 	-> [8, "8u8"]		: [  ]
		["57u8"] 	-> [9, "9u8"]		: [  ]

		["58u8"] 	-> [0, "0u8"]		: [  ]
		["64u8"] 	-> [0, "0u8"]		: [  ]

		["65u8"] 	-> [10, "10u8"]		: [  ] // 'A' - 'F'
		["66u8"] 	-> [11, "11u8"]		: [  ]
		["67u8"] 	-> [12, "12u8"]		: [  ]
		["68u8"] 	-> [13, "13u8"]		: [  ]
		["69u8"] 	-> [14, "14u8"]		: [  ]
		["70u8"] 	-> [15, "15u8"]		: [  ]

		["71u8"] 	-> [0, "0u8"]		: [  ]
		["96u8"] 	-> [0, "0u8"]		: [  ]

		["97u8"] 	-> [10, "10u8"]		: [  ] // 'a' - 'f'
		["98u8"] 	-> [11, "11u8"]		: [  ]
		["99u8"] 	-> [12, "12u8"]		: [  ]
		["100u8"] 	-> [13, "13u8"]		: [  ]
		["101u8"] 	-> [14, "14u8"]		: [  ]
		["102u8"] 	-> [15, "15u8"]		: [  ]
	]
	// Input: u8 char value. Output: u8 the numeric value represented
	"entry:"
					"echo =>sub_0"
					"const u8, 48" // '0'
	"sub_0:"		"sub Low =>0"
					"dup =>check_in_09, =>(return_in_09, jmp_not_in_09=>not_in_09=>dup_c)"
					"const u8, 10"
	"check_in_09:"	"lt =>0"
					"jmp not_in_09, jmp_not_in_09"
	"jmp_not_in_09:""ret return_in_09" // valid, return c-'0'
	"return_in_09:"

	"not_in_09:"	// =>1 is c-48
					"ret ret_0"
	"dup_c:"		"dup =>check_a, =>check_A"
					"const u8, 48"
	"check_a:"		"sub =>0"		// will only saturate to 0 if c-48 < 49 (a-48)
					"dup =>choose, =>sub_1"
					"const u8, 17" // 'A'-48
	"check_A:"		"sub Low =>choose"
	"sub_1:"   		"sub Low =>choose" // subtract the last 1 to reach 49 (a-48)
	"choose:"       "pick =>0"
					"dup =>check_in_AF, =>add_10"
					"const u8, 6"
	"check_in_AF:"	"lt =>0"
					"jmp not_in_AF, 0"
					"ret return_in_AF" // valid, return c-'A'+10
					"const u8, 10"
	"add_10:"		"add =>0"
	"return_in_AF:"

	"not_in_AF:"    // not hex digit, return 0
					"const u8, 0"
	"ret_0:"
}

test_program! {
	isxdigit [
		["0u8"] 	-> [0, "0u8"]		: [  ]
		["47u8"] 	-> [0, "0u8"]		: [  ]

		["48u8"] 	-> [1, "1u8"]		: [  ] // '0' - '9'
		["49u8"] 	-> [1, "1u8"]		: [  ]
		["50u8"] 	-> [1, "1u8"]		: [  ]
		["51u8"] 	-> [1, "1u8"]		: [  ]
		["52u8"] 	-> [1, "1u8"]		: [  ]
		["53u8"] 	-> [1, "1u8"]		: [  ]
		["54u8"] 	-> [1, "1u8"]		: [  ]
		["55u8"] 	-> [1, "1u8"]		: [  ]
		["56u8"] 	-> [1, "1u8"]		: [  ]
		["57u8"] 	-> [1, "1u8"]		: [  ]

		["58u8"] 	-> [0, "0u8"]		: [  ]
		["64u8"] 	-> [0, "0u8"]		: [  ]

		["65u8"] 	-> [1, "1u8"]		: [  ] // 'A' - 'F'
		["66u8"] 	-> [1, "1u8"]		: [  ]
		["67u8"] 	-> [1, "1u8"]		: [  ]
		["68u8"] 	-> [1, "1u8"]		: [  ]
		["69u8"] 	-> [1, "1u8"]		: [  ]
		["70u8"] 	-> [1, "1u8"]		: [  ]

		["71u8"] 	-> [0, "0u8"]		: [  ]
		["96u8"] 	-> [0, "0u8"]		: [  ]

		["97u8"] 	-> [1, "1u8"]		: [  ] // 'a' - 'f'
		["98u8"] 	-> [1, "1u8"]		: [  ]
		["99u8"] 	-> [1, "1u8"]		: [  ]
		["100u8"] 	-> [1, "1u8"]		: [  ]
		["101u8"] 	-> [1, "1u8"]		: [  ]
		["102u8"] 	-> [1, "1u8"]		: [  ]
	]
	// Input: u8 char value. Output: u8 the numeric value represented
	"entry:"
					"dup =>sub_0, =>without_bit5"
					"ret return"
					"const u8, 48" // '0'
	"sub_0:"		"sub Low =>lt_10"
					"const u8, 10"
	"lt_10:"		"lt =>digit_or_letter"

					"const u8, 223" // used to ignore bit 5, the difference between 'a' and 'A'
	"without_bit5:" "and =>sub_a"
					"const u8, 65" // 'a'
	"sub_a:"		"sub Low =>lt_6"
					"const u8, 6"
	"lt_6:"			"lt =>digit_or_letter"

	"digit_or_letter:" "or =>0"
	"return:"
}

test_program! {
	// from the C std function 'atol', but only handling the hex case
	atol_hex [
		["19u8"] 	-> [0, "0u8"]		: [  ]
		["18u8"] 	-> [16, "16u8"]		: [  ]
		["4u8"] 	-> [254, "254u8"]	: [  ]
		["5u8"] 	-> [14, "14u8"]		: [  ]
		["7u8"] 	-> [203, "203u8"]	: [  ]
		["8u8"] 	-> [11, "11u8"]		: [  ]
		["10u8"] 	-> [152, "152u8"]	: [  ]
		["11u8"] 	-> [8, "8u8"]		: [  ]
		["13u8"] 	-> [111, "111u8"]	: [  ]
		["14u8"] 	-> [15, "15u8"]		: [  ]
		["16u8"] 	-> [16, "16u8"]		: [  ] // Check correct overflow behavior
	]
	// Input: c string (*char) with hex value (without initial "0x") with null-terminator.
	// Output: u64 value (todo: right now only u8 because simulator does not support ALU of different operand types)
	"start:"
						"echo =>start_jmp_at=>entry"
						"jmp entry, start_jmp_at"
	"start_jmp_at:" //4
						".bytes u8, 102"// f
						".bytes u8, 69" // E
						".bytes u8, 0"  // null
						".bytes u8, 67" // C
						".bytes u8, 98" // b
						".bytes u8, 0"  // null
						".bytes u8, 57" // 9
						".bytes u8, 56" // 8
						".bytes u8, 0"  // null
						".bytes u8, 54" // 6
						".bytes u8, 70" // F
						".bytes u8, 0"  // null
						".bytes u8, 51" // 3
						".bytes u8, 50" // 2
						".bytes u8, 49" // 1
			/*19*/		".bytes u8, 48" // 0
						".bytes u8, 0"  // null (need 4 nulls for u64 load)
						".bytes u8, 0"  // null
						".bytes u8, 0"  // null
						".bytes u8, 0"  // null
						".bytes u8, 3"  // Non-digit
						".bytes u8, 4"  // Non-digit

	"entry:" //26
						"echo =>dup_addr"
						"const u8, 0"
						"dup =>shift_sum, =>add_sum"

	"loop_start:"
						"const u8, 16"
	"shift_sum:"		"mul Low =>add_sum" // value * 16
	"dup_addr:"			"dup =>ld_char, =>str_inc"
	"ld_char:"			"ld u8, =>0" // next char
						"dup =>isx_entry, =>check_null"
	"str_inc:"			"add Low =>loop_end=>loop_start=>dup_addr"
	"add_sum:"			"add Low =>loop_end=>loop_start=>shift_sum" // (value* 16) + hexval

						// isxdigit
	"isx_entry:"		"dup =>isx_sub_0, =>isx_without_bit5"
						"const u8, 48"
	"isx_sub_0:"		"sub Low =>0"
						"dup =>isx_lt_10, =>pre_choose_val"
						"const u8, 223"
	"isx_without_bit5:" "and =>isx_sub_a"
						"const u8, 65"
	"isx_sub_a:"		"sub Low =>0"
						"dup =>isx_lt_6, =>isx_sub_a_10"
						"const u8, 10"
	"isx_lt_10:"		"lt =>0"
						"dup =>isx_digit_or_letter, =>choose_val"
						"const u8, 6"
	"isx_lt_6:"			"lt =>isx_digit_or_letter"
	"isx_digit_or_letter:" "or =>should_loop"
						// isxdigit end

	"check_null:"		"gt =>should_loop" // check not null
	"should_loop:" 		"and =>0"
						"dup =>check_loop, =>choose_val2"
	"check_loop:"		"jmp loop_start, loop_end" // loop back as long as not NULL

						"const u8, 10"
	"isx_sub_a_10:"		"add Low =>choose_val"
	"pre_choose_val:"	"echo =>0" // ensures sub_0 is the last operand
	"choose_val:"		"pick =>pre_choose_val2"
	"pre_choose_val2:"	"const u8, 0"
	"choose_val2:"		"pick =>loop_end=>loop_start=>add_sum"

	"loop_end:"
						"ret return"
	"return:"
}
