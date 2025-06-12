use crate::TEMPORARY_DIR;
use assert_cmd::Command;
use duplicate::{duplicate_item, substitute_item};
use predicates::prelude::predicate;
use scry_asm::Assemble;
use scry_sim::{Metric, Metric::*, MetricReporter, TrackReport};
use std::{io::Write, iter::once, time::Duration};

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
	test_binary: bool,
	machine_mode: bool,
	expected_metrics: TrackReport,
) -> Result<(), Box<dyn std::error::Error>>
{
	let file_content = if test_binary
	{
		scry_asm::Raw::assemble(program.iter().cloned()).unwrap()
	}
	else
	{
		program
			.iter()
			.flat_map(|snippet| snippet.as_bytes().into_iter().chain(once(&('\n' as u8))))
			.cloned()
			.collect()
	};

	// Output program to a file
	std::fs::create_dir_all(TEMPORARY_DIR)?;
	let file = tempfile::Builder::new().tempfile_in(TEMPORARY_DIR)?;
	file.as_file().write_all(file_content.as_slice())?;

	// Run on the file with the given inputs
	let mut cmd = Command::cargo_bin("scryer")?;
	cmd.arg(file.path());
	// cmd.arg("--debug");
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
		// print!("{}",std::str::from_utf8(assert.get_output().stderr.as_slice()).
		// unwrap());
		assert.success().stdout(
			predicate::str::is_match(
				"Returned Operands(.)*?\\n".to_owned()
					+ expected_result
						// Escape any parentheses in the expected result
						.replace("(", "\\(")
						.replace(")", "\\)")
						.as_str()
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
					false, false, [$(($metric, $value),)*].into())
			}
			#[test]
			#[allow(non_snake_case)]
			fn [< $name _binary>]() -> Result<(), Box<dyn std::error::Error>>{
				test_program($program, [$($inputs,)+], $expected_machine_out, $expected_out,
					true, false, [$(($metric, $value),)*].into())
			}
			#[test]
			#[allow(non_snake_case)]
			fn [< $name _assembly_machine>]() -> Result<(), Box<dyn std::error::Error>>{
				test_program($program, [$($inputs,)+], $expected_machine_out, $expected_out,
					false, true, [$(($metric, $value),)*].into())
			}
			#[test]
			#[allow(non_snake_case)]
			fn [< $name _binary_machine>]() -> Result<(), Box<dyn std::error::Error>>{
				test_program($program, [$($inputs,)+], $expected_machine_out, $expected_out,
					true, true, [$(($metric, $value),)*].into())
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
		["0u0"] 	-> [1, "1u0"]	: [ shared_metrics([1]) ]
		["1i1"] 	-> [2, "2i1"]	: [ shared_metrics([2]) ]
		["2u2"] 	-> [3, "3u2"]	: [ shared_metrics([4]) ]
		["255u3"] 	-> [0, "256u3"]	: [ shared_metrics([8]) ]
	]
				"add Low, =>ret_at"
				"ret ret_at"
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
				"add =>0"
				"add Low, =>ret_at"
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
	[add_const_unsigned] [ "u0" ] [ "12" ] [
		["0u0"]		-> [13, "13u0"]	: [ shared_metrics ]
		["25u0"]	-> [38, "38u0"]	: [ shared_metrics ]
	];
	[add_const_signed] [ "i0" ] [ "54" ] [
	["-54i0"]	-> [1, "1i0"]	: [ shared_metrics ]
	["43i0"]	-> [98, "98i0"]	: [ shared_metrics ]
	];
	[add_const_signed_negative] [ "i0" ] [ "-2" ] [
		["-24i0"]	-> [231, "-25i0"]	: [ shared_metrics ]
		["6i0"]		-> [5, "5i0"]	: [ shared_metrics ]
	];
)]
test_program! {
	test_name [ metrics	]
				"add Low, =>add_to"
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
		["0u0"] 	-> [123, "123i0"]	: [ shared_metrics([1]) ]
		["1i1"] 	-> [234, "234u0"]	: [ shared_metrics([2]) ]
	]
					"echo =>pick_instr"
					"const u0, 234"
					"const i0, 123"
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
		["0u3"]		-> [0, "0u3"]		: [ shared_metrics([8]) ]
		["12u2"]	-> [36, "36u2"]	: [ shared_metrics([4]) ]
		["-5i1"]	-> [241, "-15i1"]	: [ shared_metrics([2]) ]
		["14i0"]	-> [42, "42i0"]		: [ shared_metrics([1]) ]
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
		["0u2","1u2","2u2"]		-> [3, "3u2"]		: [ shared_metrics([4]) ]
		["-5i2","-16i2","21i2"]	-> [0, "0i2"]		: [ shared_metrics([4]) ]
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
		["0u0"]		-> [255, "255u0"]	: [ shared_metrics([1]) ]
		["123u2"]	-> [122, "122u2"]	: [ shared_metrics([4]) ]
		["-124i1"]	-> [131, "-125i1"]	: [ shared_metrics([2]) ]
		["14i3"]	-> [13, "13i3"]		: [ shared_metrics([8]) ]
	]
				"echo =>to_add"
				// 40 nops ensure that only EchoLong can be used
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"

				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"

				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"

				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"

				"ret ret_at"
	"to_add:"	"sub Low, =>ret_at"
				"cap =>0, =>0"
				"cap =>0, =>0"
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
		["0u2","1u2"]	-> [2, "2u2"]			: [ shared_metrics([4]) ]
		["5i3","16i3"]	-> [22, "22i3"]			: [ shared_metrics([8]) ]
		["-1i0","123i0"]	-> [123, "123i0"]	: [ shared_metrics([1]) ]
	]
				"echo =>inc1, =>after_inc1=>add1"
				"ret ret_at"
				"jmp add1, after_inc1"
	"inc1:"		"add Low, =>after_inc1=>add1"
	"after_inc1:"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
				"cap =>0, =>0"
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
		["0u2","1u2"]	-> [2, "2u2"]			: [ shared_metrics([4]) ]
		["5i3","16i3"]	-> [22, "22i3"]			: [ shared_metrics([8]) ]
		["-1i0","123i0"]	-> [123, "123i0"]	: [ shared_metrics([1]) ]
	]
				"echo =>skip_at=>skip_to=>inc1, =>skip_at=>skip_to=>add1"
				"jmp skip_to, skip_at" // Skip the return
	"skip_at:"
	"jmp_to:"	"ret 0"
	"skip_to:"	"jmp jmp_to, jmp_at"
	"inc1:"		"add Low, =>add1"
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
		["0u2","1u2"]		-> [0, "0u0"]	: [ shared_metrics([4], [0]) ]
		["5i3","16i3"]		-> [0, "0u0"]	: [ shared_metrics([8], [0]) ]
		["-1i1","-1i1"]		-> [1, "1u0"]	: [ shared_metrics([2], [1]) ]
		["4i0","4i0"]		-> [1, "1u0"]	: [ shared_metrics([1], [1]) ]
	]
	// Return 1 if they are equal, 0 otherwise
				"sub Low, =>0"
				"jmp if_equal, 0"
	"if_unequal:"
				"ret 1"
				"const u0, 0"
	"if_equal:"
				"ret 1"
				"const u0, 1"
}

#[substitute_item(
	shared_metrics(extra_loops) [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 5 + (4 * extra_loops)
		ConsumedBytes		: 5 + (4 * extra_loops)
		QueuedValues		: 7 + (4 * extra_loops)
		QueuedValueBytes	: 7 + (4 * extra_loops)
		InstructionReads	: 12 + (5 * extra_loops)
		IssuedBranches		: 0 + (1 * extra_loops)
		TriggeredBranches	: 0 + (1 * extra_loops)
		ReorderedOperands	: 7 + (2 * extra_loops)
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
	"entry:"
							"dup 	=>dec_n, =>0"								// Send to next jmp, and decrementor
							"jmp	early_ret, 0"								// If n=0, result is 0

							"const u0, 0"										// Initial values
							"const u0, 1"
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
							"cap =>2, =>0"										// Get low value as result, throw high out
	"final_ret_trig:"
	"early_ret:" 			"ret early_ret_trig"								// n=0, return 0
							"const u0, 0"
	"early_ret_trig:"
}

#[substitute_item(
	shared_metrics [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 1
		ConsumedBytes		: 1
		QueuedValues		: 0
		QueuedValueBytes	: 0
		QueuedReads			: 1
		DataReads			: 0
		DataReadBytes		: 0
		InstructionReads	: 3
	];
)]
test_program! {
	load_from_array [
		["6u0"] 	-> [123, "Load(Regular,0x6,123u0)"]	: [ shared_metrics ]
		["7u0"] 	-> [124, "Load(Regular,0x7,124u0)"]	: [ shared_metrics ]
		["8u0"] 	-> [125, "Load(Regular,0x8,125u0)"]	: [ shared_metrics ]
		["9u0"] 	-> [126, "Load(Regular,0x9,126u0)"]	: [ shared_metrics ]
	]
				"ld u0"
				"echo =>ret_at"
				"ret ret_at"
	"ret_at:"
	"load_from:"
				".bytes u0, 123"
				".bytes u0, 124"
				".bytes u0, 125"
				".bytes u0, 126"
}

#[substitute_item(
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
	consumed_loaded [
		["6u0"] 	-> [121, "121u0"]	: [ shared_metrics ]
		["7u0"] 	-> [122, "122u0"]	: [ shared_metrics ]
		["8u0"] 	-> [123, "123u0"]	: [ shared_metrics ]
		["9u0"] 	-> [124, "124u0"]	: [ shared_metrics ]
	]
				"ld u0"
				"add Low, =>1"
				"ret 0"
	"data:"
				".bytes u0, 120"
				".bytes u0, 121"
				".bytes u0, 122"
				".bytes u0, 123"

}

#[substitute_item(
	shared_metrics(addr_size) [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		IssuedBranches		: 1
		TriggeredBranches	: 1
		ConsumedOperands	: 11
		ConsumedBytes		: 10 + addr_size
		QueuedValues		: 10
		QueuedValueBytes	: 10
		QueuedReads			: 1
		DataReads			: 0
		DataReadBytes		: 0
		ReorderedOperands	: 9
		InstructionReads	: 18
	];
)]
test_program! {
	load_before_store [
		["8u0"] 	-> [0, "Load(Regular,0x8,0u0)"]	: [ shared_metrics([1]) ]
		["9u1"] 	-> [1, "Load(Regular,0x9,1u0)"]	: [ shared_metrics([2]) ]
		["10u2"] 	-> [2, "Load(Regular,0xA,2u0)"]	: [ shared_metrics([4]) ]
		["11u3"] 	-> [3, "Load(Regular,0xB,3u0)"]	: [ shared_metrics([8]) ]
	]
				"ld u0"
				"echo =>data=>init_data=>ret_at"
				"ret ret_at"
				"jmp init_data, 0"
	"data:"
				".bytes u0, 255"
				".bytes u0, 255"
	"data_3:"	".bytes u0, 255"
				".bytes u0, 255"

	"init_data:"
				// Initialize data array to [0,1,...]
				"const u0, data" // Absolute addressing
				"const u0, 0"
				"st"
				"const u0, 1" // Indexed absolute addressing
				"const u0, data"
				"const u0, 1"
				"st"
				"const i0, store_3=>data_3" // Relative addressing
				"const u0, 2"
	"store_3:"	"st"
				"const u0, 3" // Relative indexed addressing
				"const i0, store_4=>data"
				"const u0, 3"
	"store_4:"	"st"
	"ret_at:"
}

#[substitute_item(
	shared_metrics [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 5
		ConsumedBytes		: 5
		QueuedValues		: 4
		QueuedValueBytes	: 4
		QueuedReads			: 1
		DataReads			: 0
		DataReadBytes		: 0
		InstructionReads	: 6
		ReorderedOperands	: 2
	];
)]
#[duplicate_item(
	[
		name [load_from_absolute_address]
		tests [
			["0u0"] 	-> [45, "Load(Regular,0x16,45i1)"]	: [ shared_metrics ]
			["1u0"] 	-> [46, "Load(Regular,0x18,46i1)"]	: [ shared_metrics ]
			["2u0"] 	-> [47, "Load(Regular,0x1A,47i1)"]	: [ shared_metrics ]
			["3u0"] 	-> [48, "Load(Regular,0x1C,48i1)"]	: [ shared_metrics ]
		]
		addr_type ["u0"]
		addr_val ["22"]
	]
	[
		name [load_from_label_address]
		tests [
			["0u0"] 	-> [45, "Load(Regular,0x16,45i1)"]	: [ shared_metrics ]
			["2u0"] 	-> [47, "Load(Regular,0x1A,47i1)"]	: [ shared_metrics ]
			["5u0"] 	-> [50, "Load(Regular,0x20,50i1)"]	: [ shared_metrics ]
			["7u0"] 	-> [52, "Load(Regular,0x24,52i1)"]	: [ shared_metrics ]
		]
		addr_type ["u0"]
		addr_val ["data"]
	]
	[
		name [load_from_relative_address]
		tests [
			["1i0"] 	-> [46, "Load(Regular,0x18,46i1)"]	: [ shared_metrics ]
			["3i0"] 	-> [48, "Load(Regular,0x1C,48i1)"]	: [ shared_metrics ]
			["5i0"] 	-> [50, "Load(Regular,0x20,50i1)"]	: [ shared_metrics ]
			["7i0"] 	-> [52, "Load(Regular,0x24,52i1)"]	: [ shared_metrics ]
		]
		addr_type ["i0"]
		addr_val ["12"]
	]
	[
		name [load_from_relative_labels]
		tests [
			["0i0"] 	-> [45, "Load(Regular,0x16,45i1)"]	: [ shared_metrics ]
			["2i0"] 	-> [47, "Load(Regular,0x1A,47i1)"]	: [ shared_metrics ]
			["4i0"] 	-> [49, "Load(Regular,0x1E,49i1)"]	: [ shared_metrics ]
			["6i0"] 	-> [51, "Load(Regular,0x22,51i1)"]	: [ shared_metrics ]
		]
		addr_type ["i0"]
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
	"load:"		"ld i1"
				".bytes i1, -1"
				".bytes i1, -1"
				".bytes i1, -1"
				".bytes i1, -1"
				".bytes i1, -1"
	"data:"
				".bytes i1, 45"
				".bytes i1, 46"
				".bytes i1, 47"
				".bytes i1, 48"
				".bytes i1, 49"
				".bytes i1, 50"
				".bytes i1, 51"
				".bytes i1, 52"

}

#[substitute_item(
	shared_metrics [
		IssuedReturns		: 1
		IssuedBranches		: 1
		TriggeredReturns	: 1
		TriggeredBranches	: 1
		ConsumedOperands	: 2
		ConsumedBytes		: 2
		QueuedValues		: 1
		QueuedValueBytes	: 1
		QueuedReads			: 1
		DataReads			: 0
		DataReadBytes		: 0
		InstructionReads	: 5
		ReorderedOperands	: 2
	];
)]
test_program! {
	load_from_relative_indexed [
		["0u0"] 	-> [45, "Load(Regular,0xC,45i1)"]	: [ shared_metrics ]
		["2u0"] 	-> [46, "Load(Regular,0x10,46i1)"]	: [ shared_metrics ]
		["5u0"] 	-> [47, "Load(Regular,0x16,47i1)"]	: [ shared_metrics ]
		["7u0"] 	-> [48, "Load(Regular,0x1A,48i1)"]	: [ shared_metrics ]
	]
				// Jump past data
				"echo =>jmp_at=>start=>1"
				"jmp start, 0"
	"jmp_at:"
				".bytes i1, -1"
				".bytes i1, -1"
				".bytes i1, -1"
				".bytes i1, -1"
	"data:"
				".bytes i1, 45"
				".bytes i1, -1"
				".bytes i1, 46"
				".bytes i1, -1"
				".bytes i1, -1"
				".bytes i1, 47"
				".bytes i1, -1"
				".bytes i1, 48"

				".bytes i1, -1"
				".bytes i1, -1"
				".bytes i1, -1"
				".bytes i1, -1"
	"start:"	"ret 2"
				"const i0, load=>data"
	"load:"		"ld i1"
}

#[substitute_item(
	shared_metrics(len) [
		IssuedReturns		: 1
		TriggeredReturns	: 1
		ConsumedOperands	: 3
		ConsumedBytes		: 8 + (len*2)
		QueuedValues		: 2
		QueuedValueBytes	: 8 + len
		QueuedReads			: 1
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
		["0u0"] 	-> [2, "2u3"]	: [ shared_metrics([1]) ]
		["12u1"] 	-> [14, "14u3"]	: [ shared_metrics([2]) ]
		["23u2"] 	-> [25, "25u3"]	: [ shared_metrics([4]) ]
		["45u3"] 	-> [47, "47u3"]	: [ shared_metrics([8]) ]
	]
				"add Low, =>store"
				"rsrv 64"
	"store:"
				"st [0]"
				"ret end"
				"ld u3 [0]"
				"add Low, =>end"
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
		QueuedValues		: 4
		QueuedValueBytes	: 13
		QueuedReads			: 1
		DataReads			: 1
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
		["123u2"] 	-> [126, "126u2"]	: [ shared_metrics ]
		["75u2"] 	-> [78, "78u2"]		: [ shared_metrics ]
		["46u2"] 	-> [49, "49u2"]		: [ shared_metrics ]
	]
				"add Low, =>store"
				"rsrv 16"
				"rsrv 8, Base"
	"store:"
				"st [2]"
				"const u0, func1"
				"call 0"
				"add Low, =>end"
				"ret end"
				"free 16"
	"end:"
	"func1:"
				"ret func1_end"
				"ld u2 [0]"
				"add Low, =>func1_end"
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
		QueuedValues		: 4
		QueuedValueBytes	: 7
		QueuedReads			: 1
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
		["123u1"] 	-> [126, "126u1"]	: [ shared_metrics ]
		["75u1"] 	-> [78, "78u1"]		: [ shared_metrics ]
		["46u1"] 	-> [49, "49u1"]		: [ shared_metrics ]
	]
				"add Low, =>call_args"
				"const u0, func1"
				"call 0"
	"call_args:"
				"ld u1 [0]"
				"add Low, =>end"
				"ret end"
				"free 2"
	"end:"
	"func1:"
				"add Low, =>store"
				"rsrv 2, Base"
	"store:"	"st [0]"
				"ret func1_end"
	"func1_end:"
}

#[substitute_item(
	shared_metrics(len) [
		IssuedBranches		: len-1
	    IssuedReturns		: 1
	    TriggeredBranches	: len-1
	    TriggeredReturns	: 1
	    ConsumedOperands	: len*7
	    ConsumedBytes		: len*7
	    QueuedValues		: 3+(len*6)
	    QueuedValueBytes	: 3+(len*6)
	    QueuedReads			: len*2
	    ReorderedOperands	: 5+(len*7)
	    InstructionReads	: 6+(len*10)
	    DataReads			: len
	    DataReadBytes		: len
	];
)]
test_program! {
	find_max [
		["32u0", "1u0"] -> [0, "0u0"]		: [ shared_metrics([1]) ]
		["32u0", "2u0"] -> [1, "1u0"]		: [ shared_metrics([2]) ]
		["34u0", "4u0"] -> [4, "4u0"]		: [ shared_metrics([4]) ]
		["39u0", "3u0"]	-> [99, "99u0"]		: [ shared_metrics([3]) ]
		["38u0", "7u0"]	-> [207, "207u0"]	: [ shared_metrics([7]) ]
	]
				// Takes as input (1) the address of an u0-array, (2) its length
				// returns the highest value in the array. (Array needn't be sorted)
	"start:"
					"echo =>dup_addr, =>dec_size"
					"const u0, 0"					// Initial max
					"dup =>pre_pick, =>compare"
					"ret return"
	"dup_addr:"		"dup =>load_next, =>addr_repeat"
	"loop_start:"
	"dec_size:"		"sub Low, =>0"
					"dup =>load_next, =>loop_end=>loop_start=>dec_size, =>"
	"loop_cond:"	"jmp loop_start, loop_end"
	"load_next:"	"ld u0"
					"dup =>compare, =>pre_pick"
	"addr_repeat:"	"dup =>loop_end=>loop_start=>load_next,"
						 "=>loop_end=>loop_start=>addr_repeat"
	"compare:"		"sub High, =>pick_max"		// Do (max - new), and if carry is 1 new is higher.
												// We dont need the lower order output
	"pre_pick:"		"echo =>0"					// Pick either previous max (1) or new value (2)
	"pick_max:"		"pick =>0"					// based on comparison (0)
					"dup =>loop_end=>loop_start=>compare,"	// Send max to next iteration's compare and pick
						"=>loop_end=>loop_start=>pre_pick"
	"loop_end:"		"cap =>6, =>0"		// get final max for return
	"return:"

	// Address: 32
	"data_1:"	".bytes u0, 0"
				".bytes u0, 1"
	// Address: 34
	"data_2:"	".bytes u0, 4"
				".bytes u0, 1"
				".bytes u0, 0"
				".bytes u0, 2"
	// Address: 38
	"data_3:"	".bytes u0, 103"
				".bytes u0, 99"
				".bytes u0, 0"
				".bytes u0, 4"
				".bytes u0, 207"
				".bytes u0, 168"
				".bytes u0, 104"
}

test_program! {
	memcpy [
		["255u0", "255u0", "0u0"]	->
			[0, "Load(Regular,0x4C,0i0), Load(Regular,0x4D,0i0), Load(Regular,0x4E,0i0), Load(Regular,0x4F,0i0)"] : []
		["75u0", "76u0", "1u0"]		->
			[7, "Load(Regular,0x4C,7i0), Load(Regular,0x4D,0i0), Load(Regular,0x4E,0i0), Load(Regular,0x4F,0i0)"] : []
		["74u0", "77u0", "2u0"]		->
			[0, "Load(Regular,0x4C,0i0), Load(Regular,0x4D,5i0), Load(Regular,0x4E,7i0), Load(Regular,0x4F,0i0)"] : []
		["73u0", "77u0", "3u0"]		->
			[0, "Load(Regular,0x4C,0i0), Load(Regular,0x4D,3i0), Load(Regular,0x4E,5i0), Load(Regular,0x4F,7i0)"] : []
		["72u0", "76u0", "4u0"]		->
			[2, "Load(Regular,0x4C,2i0), Load(Regular,0x4D,3i0), Load(Regular,0x4E,5i0), Load(Regular,0x4F,7i0)"] : []
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
	"load_next:"		"ld u0"
						"echo =>store_copy"
	"dec_count:"		"sub Low, =>0"
						"dup =>loop_cond, =>loop_end=>loop_start=>dec_count"
	"loop_cond:"		"jmp loop_start, loop_end"
	"dup_sink:"			"dup =>store_copy, =>inc_sink"
	"inc_source:"		"add Low, =>0"
						"dup =>loop_end=>loop_start=>load_next,"
							"=>loop_end=>loop_start=>inc_source"
	"inc_sink:"			"add Low, =>loop_end=>loop_start=>dup_sink"
	"store_copy:"		"st"
	"loop_end:"
						"cap =>0, =>0"				// Throw out values surviving the loop exit
						"cap =>0, =>0"
						"cap =>0, =>0"
						"cap =>0, =>0"
						"cap =>0, =>0"
						"cap =>0, =>0"
						"cap =>0, =>0"
						"cap =>0, =>0"
						"cap =>0, =>0"
						"cap =>0, =>0"

						"const u0, dst1"
						"ld i0"
						"echo =>return"
						"const u0, dst2"
						"ld i0"
						"echo =>return"
						"const u0, dst3"
						"ld i0"
						"echo =>return"
						"const u0, dst4"
						"ld i0"
	"return:"
	// Address: 72
	"src:"
						".bytes i0, 2"
						".bytes i0, 3"
						".bytes i0, 5"
						".bytes i0, 7"

	"dst1:"				".bytes i0, 0"
	"dst2:"				".bytes i0, 0"
	"dst3:"				".bytes i0, 0"
	"dst4:"				".bytes i0, 0"
}

test_program! {
	strcpy [
		["63u0", "60u0"]	->
			[255, "Load(Regular,0x3E,255u0), Load(Regular,0x3F,223u0), Load(Regular,0x40,0u0), Load(Regular,0x41,255u0)"] : []
		["63u0", "59u0"]	->
			[255, "Load(Regular,0x3E,255u0), Load(Regular,0x3F,211u0), Load(Regular,0x40,223u0), Load(Regular,0x41,0u0)"] : []
		["62u0", "58u0"]	->
			[199, "Load(Regular,0x3E,199u0), Load(Regular,0x3F,211u0), Load(Regular,0x40,223u0), Load(Regular,0x41,0u0)"] : []
	]
					// Takes destination address and source address.
					// Copies the string from source to destination including final null character
	"start:"
						"echo =>dup_dst,  =>dup_src"
						"ret return_at"

	"loop_start:"
	"dup_src:"			"dup =>load, =>inc_src"
	"load:"				"ld u0"
						"dup =>loop_cond, =>store"
	"loop_cond:"		"jmp loop_start, loop_end"
	"dup_dst:"			"dup =>store, =>0"
						"add Low, =>loop_end=>loop_start=>dup_dst"
	"inc_src:"			"add Low, =>loop_end=>loop_start=>dup_src"
	"store:"			"st"
	"loop_end:"
						"cap =>0, =>0"// Throw out values surviving the loop exit
						"cap =>0, =>0"
						"cap =>0, =>0"
						"cap =>0, =>0"
						"cap =>0, =>0"
						"cap =>0, =>0"
						"cap =>0, =>0"
						"cap =>0, =>0"

						"const u0, dst1"
						"ld u0"
						"echo =>return_at"
						"const u0, dst2"
						"ld u0"
						"echo =>return_at"
						"const u0, dst3"
						"ld u0"
						"echo =>return_at"
						"const u0, dst4"
						"ld u0"
	"return_at:"
	// Address: 58
	"src:"
						".bytes u0, 199"
						".bytes u0, 211"
						".bytes u0, 223"
						".bytes u0, 0"

	"dst1:"				".bytes u0, 255"
	"dst2:"				".bytes u0, 255"
	"dst3:"				".bytes u0, 255"
	"dst4:"				".bytes u0, 255"
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
		QueuedReads			: 0
		ReorderedOperands	: 3
		InstructionReads	: 8
		DataReads			: 0
		DataReadBytes		: 0
	];
)]
test_program! {
	simple_call [
		["0u0"] 	-> [3, "3u0"]		: [ shared_metrics ]
		["1u0"] 	-> [4, "4u0"]		: [ shared_metrics ]
		["2u0"] 	-> [5, "5u0"]		: [ shared_metrics ]
		["244u0"]	-> [247, "247u0"]	: [ shared_metrics ]
	]

	"entry:"
					"echo =>call_inputs"
					"const u0, addr_three"
					"call 0"
	"call_inputs:"
					"echo =>return_at"
					"ret return_at"
	"return_at:"

	"addr_three:"
					"const u0, 3"
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
		QueuedReads			: 0
		ReorderedOperands	: 1
		InstructionReads	: 7
		DataReads			: 0
		DataReadBytes		: 0
	];
)]
test_program! {
	queue_past_call [
		["0u0"] 	-> [3, "3u0"]		: [ shared_metrics ]
		["1u0"] 	-> [4, "4u0"]		: [ shared_metrics ]
		["2u0"] 	-> [5, "5u0"]		: [ shared_metrics ]
		["244u0"]	-> [247, "247u0"]	: [ shared_metrics ]
	]

	"entry:"
					"echo =>|=>add_result"
					"const u0, fn_return_three"
					"call 0"
	"call_inputs:"
	"add_result:"	"add =>return_at"
					"ret return_at"
	"return_at:"

	"fn_return_three:"
					"ret return_at2"
					"const u0, 3"
	"return_at2:"
}

test_program! {
	cmp_i8 [
		["0u0", "0u0"] 	-> [0, "0i0"]		: []
		["0u0", "1u0"] 	-> [142, "-114i0"]	: []
		["1u0", "0u0"] 	-> [114, "114i0"]	: []
		["0u0", "2u0"] 	-> [128, "-128i0"]	: []
	]

	"entry:"
								"echo =>entry_add1, =>entry_add2"
								// Convert indices to pointers into the array
								"const u0, data_i8"
	"entry_add1:"				"add =>fn_cmp_i8_args"
								"const u0, data_i8"
	"entry_add2:"				"add =>fn_cmp_i8_args"

								// Call
								"const u0, fn_cmp_i8"
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
	"fn_cmp_i8_ld1:"			"ld i0"
								"echo =>fn_cmp_i8_sub"
	"fn_cmp_i8_ld2:"			"ld i0"
	"fn_cmp_i8_sub:"			"sub =>0"
	"fn_cmp_i8_ret:"

	"data_i8:"
								".bytes i0, -124"
								".bytes i0, -10"
								".bytes i0, 10"

}

test_program! {
	bsearch [
		// Empty array
		["2i0", "0u0", "1u0"] 	-> [0, "0u0"]		: []
		["2i1", "0u0", "2u0"] 	-> [0, "0u0"]		: []
		// 1 element array
		["-1i0", "1u0", "1u0"] 	-> [0, "0u0"]		: []
		["0i0", "1u0", "1u0"] 	-> [4, "4u0"]		: []
		["7i0", "1u0", "1u0"] 	-> [0, "0u0"]		: []
		["-1i0", "1u0", "2u0"] 	-> [0, "0u0"]		: []
		["0i0", "1u0", "2u0"] 	-> [8, "8u0"]		: []
		["7i0", "1u0", "2u0"] 	-> [0, "0u0"]		: []
		// 2 element array
		["-1i0", "2u0", "1u0"] 	-> [0, "0u0"]		: []
		["0i0", "2u0", "1u0"] 	-> [4, "4u0"]		: []
		["2i0", "2u0", "1u0"] 	-> [5, "5u0"]		: []
		["7i0", "2u0", "1u0"] 	-> [0, "0u0"]		: []
		["-1i0", "2u0", "2u0"] 	-> [0, "0u0"]		: []
		["0i0", "2u0", "2u0"] 	-> [8, "8u0"]		: []
		["2i0", "2u0", "2u0"] 	-> [10, "10u0"]		: []
		["7i0", "2u0", "2u0"] 	-> [0, "0u0"]		: []
		// 3 element array
		["-1i0", "3u0", "1u0"] 	-> [0, "0u0"]		: []
		["0i0", "3u0", "1u0"] 	-> [4, "4u0"]		: []
		["2i0", "3u0", "1u0"] 	-> [5, "5u0"]		: []
		["4i0", "3u0", "1u0"] 	-> [6, "6u0"]		: []
		["7i0", "3u0", "1u0"] 	-> [0, "0u0"]		: []
		["-1i0", "3u0", "2u0"] 	-> [0, "0u0"]		: []
		["0i0", "3u0", "2u0"] 	-> [8, "8u0"]		: []
		["2i0", "3u0", "2u0"] 	-> [10, "10u0"]		: []
		["4i0", "3u0", "2u0"] 	-> [12, "12u0"]		: []
		["7i0", "3u0", "2u0"] 	-> [0, "0u0"]		: []
		// 4 element array
		["-1i0", "4u0", "1u0"] 	-> [0, "0u0"]		: []
		["0i0", "4u0", "1u0"] 	-> [4, "4u0"]		: []
		["2i0", "4u0", "1u0"] 	-> [5, "5u0"]		: []
		["4i0", "4u0", "1u0"] 	-> [6, "6u0"]		: []
		["6i0", "4u0", "1u0"] 	-> [7, "7u0"]		: []
		["7i0", "4u0", "1u0"] 	-> [0, "0u0"]		: []
		["-1i0", "4u0", "2u0"] 	-> [0, "0u0"]		: []
		["0i0", "4u0", "2u0"] 	-> [8, "8u0"]		: []
		["2i0", "4u0", "2u0"] 	-> [10, "10u0"]		: []
		["4i0", "4u0", "2u0"] 	-> [12, "12u0"]		: []
		["6i0", "4u0", "2u0"] 	-> [14, "14u0"]		: []
		["7i0", "4u0", "2u0"] 	-> [0, "0u0"]		: []
		// 5 element array
		["-1i0", "5u0", "2u0"] 	-> [0, "0u0"]		: []
		["0i0", "5u0", "2u0"] 	-> [8, "8u0"]		: []
		["1i0", "5u0", "2u0"] 	-> [0, "0u0"]		: []
		["2i0", "5u0", "2u0"] 	-> [10, "10u0"]		: []
		["3i0", "5u0", "2u0"] 	-> [0, "0u0"]		: []
		["4i0", "5u0", "2u0"] 	-> [12, "12u0"]		: []
		["5i0", "5u0", "2u0"] 	-> [0, "0u0"]		: []
		["6i0", "5u0", "2u0"] 	-> [14, "14u0"]		: []
		["7i0", "5u0", "2u0"] 	-> [0, "0u0"]		: []
		["8i0", "5u0", "2u0"] 	-> [16, "16u0"]		: []
		["9i0", "5u0", "2u0"] 	-> [0, "0u0"]		: []
	]
								"echo =>1"
								"jmp entry, 0"
	/////////////////////////////////////////////////////////////////////////////////////////////
	// Array for i8 data
	"data_i8:" // Addr: 4
								".bytes i0, 0"
								".bytes i0, 2"
								".bytes i0, 4"
								".bytes i0, 6"
	/////////////////////////////////////////////////////////////////////////////////////////////
	// Array for i16 data
	"data_i16:" // Addr: 8
								".bytes i1, 0"
								".bytes i1, 2"
								".bytes i1, 4"
								".bytes i1, 6"
								".bytes i1, 8"
	/////////////////////////////////////////////////////////////////////////////////////////////
	// Addresses of comparison functions
	"cmp_fns:"// Addr: 16
								".bytes u0, fn_cmp_i8"
								".bytes u0, fn_cmp_i16"
	/////////////////////////////////////////////////////////////////////////////////////////////
	// Addresses of arrays
	"data_addr:" // Addr: 18
								".bytes u0, data_i8"
								".bytes u0, data_i16"
	/////////////////////////////////////////////////////////////////////////////////////////////
	// Store position for key to allow for getting its address, which is passed to bsearch
	"key_store:" // Addr: 20
								".bytes u1, 0"
	/////////////////////////////////////////////////////////////////////////////////////////////
	// bsearch comparison of i8 pointers
	"fn_cmp_i8:" // Addr: 22
								"echo =>fn_cmp_i8_ld1, =>fn_cmp_i8_ld2"
								"ret fn_cmp_i8_ret"
	"fn_cmp_i8_ld1:"			"ld i0"
								"echo =>fn_cmp_i8_sub"
	"fn_cmp_i8_ld2:"			"ld i0"
	"fn_cmp_i8_sub:"			"sub =>0"
	"fn_cmp_i8_ret:"
	/////////////////////////////////////////////////////////////////////////////////////////////
	// bsearch comparison of i16 pointers
	"fn_cmp_i16:" // Addr: 34
								"echo =>fn_cmp_i16_ld1, =>fn_cmp_i16_ld2"
								"ret fn_cmp_i16_ret"
	"fn_cmp_i16_ld1:"			"ld i1"
								"echo =>fn_cmp_i16_sub"
	"fn_cmp_i16_ld2:"			"ld i1"
	"fn_cmp_i16_sub:"			"sub =>0"
	"fn_cmp_i16_ret:"
	/////////////////////////////////////////////////////////////////////////////////////////////
	// Test setup
	"entry:"
								"echo =>entry_store_key, =>entry_after_base, =>"
								"dup =>echo_size, =>sub_size"
								// First store the key, so that we can get its address
								"const u0, key_store"
	"entry_store_key:"			"st"
								// Choose comparison function and array based on size
	"sub_size:"					"sub Low, =>0"
								"dup =>calc_cmp_fn, =>calc_data_addr"
								// Choose data array
								"const u0, data_addr"
	"calc_data_addr:"			"add =>0"
								"ld u0"
								// Pass base address, nitems, and size to bsearch
								"echo =>entry_after_key"
	"entry_after_base:"			"echo =>entry_after_key"
	"echo_size:"				"echo =>entry_after_key"
								// Choose comparison function and put its address on the stack
								"const u0, cmp_fns"
	"calc_cmp_fn:"				"add =>0"
								"ld u0"
								"echo =>entry_store_cmp_fn"
								"rsrv 2"
	"entry_store_cmp_fn:"		"st [0]"
								// Call bsearch
								"const u0, fn_bsearch"
								"call call_args"
														//cmp_fn (stack)
														// size
														// nitems
														// base
	"entry_after_key:"			"const u0, key_store"	// key
	"call_args:"
								"echo =>entry_ret"
								"ret entry_ret"
	"entry_ret:"
								".bytes u2, 0"
	/////////////////////////////////////////////////////////////////////////////////////////////
	// bsearch
	"fn_bsearch:"
										"echo =>fn_bsearch_dup_key, =>fn_bsearch_dup_base, =>"
										"echo =>fn_bsearch_dup_nr, =>fn_bsearch_dup_size"

	"fn_bsearch_dup_nr:"				"dup =>fn_bsearch_dup_nr2, =>fn_bsearch_check_zero"
										// If elements == 0, return null
	"fn_bsearch_check_zero:"			"jmp fn_bsearch_null, fn_bsearch_loop_start"
										"free 2, Base" // free stack so nothing is returned

	"fn_bsearch_loop_start:"
	"fn_bsearch_dup_nr2:"				"dup =>fn_bsearch_pivot_half_elements, =>|=>fn_bsearch_dec_nr, =>"
	
										// Check iteration
	"fn_bsearch_check_loop:"			"jmp fn_bsearch_loop_start, fn_bsearch_loop_end"
	
										// Issue call to comparison function
										// these instruction are here to match with fn_bsearch_null
										// So no operands from the loop are returned
										"ld u0 [0]"
										"call fn_bsearch_cmp_call_args"
	
	
	"fn_bsearch_dup_key:"				"dup =>fn_bsearch_cmp_call_args, =>|=>fn_bsearch_loop_end=>fn_bsearch_loop_start=>fn_bsearch_dup_key"
	"fn_bsearch_dup_base:"				"dup =>fn_bsearch_pivot_add, =>|=>fn_bsearch_cap_base"
	
										// Calculate the pivot (element to compare)
	"fn_bsearch_pivot_half_elements:"	"shr =>fn_bsearch_pivot_scale"
	"fn_bsearch_dup_size:"				"dup =>|=>fn_bsearch_calc_right_base, =>|=>fn_bsearch_loop_end=>fn_bsearch_loop_start=>fn_bsearch_dup_size, =>"
	"fn_bsearch_pivot_scale:"			"mul Low, =>fn_bsearch_pivot_add"
	"fn_bsearch_pivot_add:"				"add Low, =>fn_bsearch_pivot_dup"
	"fn_bsearch_pivot_dup:"				"dup =>fn_bsearch_cmp_call_args,\
											 =>|=>(\
											 	fn_bsearch_calc_right_base, \
											 	fn_bsearch_check_jmp_loc=>fn_bsearch_equal=>fn_bsearch_equal_cap\
											)"
	
										// Trigger call
	"fn_bsearch_cmp_call_args:"
										"dup =>fn_bsearch_check_equal, =>fn_bsearch_check_positive"
	
										// if 0, return pivot
	"fn_bsearch_check_equal:"			"jmp fn_bsearch_equal, fn_bsearch_check_jmp_loc"
	"fn_bsearch_check_positive:"		"gt =>fn_bsearch_choose_base"
	
										// decrement nr, then halve
	"fn_bsearch_dec_nr:"				"sub =>fn_bsearch_halve_nr"
	"fn_bsearch_halve_nr:"				"shr =>fn_bsearch_loop_end=>fn_bsearch_loop_start=>fn_bsearch_dup_nr2"
	
	"fn_bsearch_check_jmp_loc:"
	
										// Calculate move right base
	"fn_bsearch_cap_base:"				"echo =>fn_bsearch_choose_base"
	"fn_bsearch_calc_right_base:"		"add Low, =>fn_bsearch_choose_base" // pivot + size
	"fn_bsearch_choose_base:"			"pick =>fn_bsearch_loop_end=>fn_bsearch_loop_start=>fn_bsearch_dup_base"

	"fn_bsearch_loop_end:"

	"fn_bsearch_null:"					// nothing found
										"ret fn_bsearch_null_end"
										// Return NULL
										"const u0, 0"
	"fn_bsearch_null_end:"
	"fn_bsearch_end:"

	"fn_bsearch_equal:"					// Pivot is what we are looking for, return its addr
										"ret fn_bsearch_equal_end"
	"fn_bsearch_equal_cap:"				"echo =>fn_bsearch_equal_pick" // capture size + pivot
										"nop"	// ignore in-flight operands
										"nop"
	"fn_bsearch_equal_pick:"			"const u0, 1"					// substitute for immediate pick
										"pick =>fn_bsearch_equal_end"
										// Return pivot address
	"fn_bsearch_equal_end:"

}
