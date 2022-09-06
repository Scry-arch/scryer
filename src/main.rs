mod simulator;

use std::collections::HashMap;
use clap::Parser;
use scryasm::Assemble;
use scry_sim::{BlockedMemory, CallFrameState, ExecState, Executor, OperandState};

/// Command-line arguments
#[derive(Parser)]
struct Cli {
    /// The path to the file to execute
    #[clap(parse(from_os_str))]
    path: std::path::PathBuf,
    
    /// Signals that the file is in assembly
    #[clap(short, long)]
    assembly: bool,
}

fn main() {
    let args = Cli::parse();
    
    let contents = std::fs::read_to_string(args.path).unwrap();
    
    // Assume file is assembly
    let program = scryasm::Raw::assemble(std::iter::once(contents.as_str())).unwrap();
    let original_state = ExecState{
        address: 0,
        frame: CallFrameState{
            ret_addr: 0,
            branches: HashMap::new(),
            op_queues: HashMap::from([(0,(OperandState::Ready(2u32.into()), Vec::new()))]),
            reads: Vec::new()
        },
        frame_stack: vec![
            CallFrameState{
                ret_addr: 0,
                branches: HashMap::new(),
                op_queues: HashMap::new(),
                reads: Vec::new()
            }
        ]
    };
    let mut res = Executor::from_state(&original_state, BlockedMemory::new(program, 0)).step(&mut ());
    while res.is_ok() {
        let exec = res.unwrap();
        let state = exec.state();
        if state.frame_stack.len() == 0 {
            // Done
            if let Some((OperandState::Ready(v),_)) = state.frame.op_queues.get(&0) {
                std::process::exit(v.iter().next().unwrap().bytes().map_or(123, |b| b[0] as i32));
            } else {
                std::process::exit(123);
            }
        }
        res = exec.step(&mut ());
    }
}
