mod ir;
mod sim;

use std::ffi::CString;
use ir::types::*;

type Value = isize;

use clap::Parser;
#[derive(Parser, Debug)]
struct Cli {
    file:String,

    #[arg(long)]
    eval:bool
}

unsafe extern "C" {
    fn caml_startup(argv: *mut *mut i8);
    fn caml_named_value(name: *const i8) -> *const Value;
    fn caml_callback_exn(closure: Value, arg: Value) -> Value;
    fn caml_copy_string(s: *const i8) -> Value;
    fn caml_string_length(s: Value) -> usize;
}

fn is_exception_result(v: Value) -> bool {
    (v & 3) == 2
}

fn ocaml_call(name: &str, arg: Value) -> Value {
    unsafe {
        let cname = CString::new(name).unwrap();
        let closure = caml_named_value(cname.as_ptr());
        let result = caml_callback_exn(*closure, arg);
        if is_exception_result(result) {
            panic!("OCaml exception in {}", name);
        }
        result
    }
}

fn ocaml_string(v: Value) -> String {
    unsafe {
        let ptr = v as *const u8;
        let len = caml_string_length(v);
        let bytes = std::slice::from_raw_parts(ptr, len);
        String::from_utf8_lossy(bytes).into_owned()
    }
}

fn main() {
    // cargo run -- ../anvil/examples/cache.anvil 
    let cli = Cli::parse();
    let filename = cli.file;
    let is_json_output_only = cli.eval;

    unsafe {
        let arg0 = CString::new("anvil-sim").unwrap();  
        let mut argv = [arg0.as_ptr() as *mut _, std::ptr::null_mut()];
        caml_startup(argv.as_mut_ptr());

        let ocaml_filename = caml_copy_string(
            CString::new(filename.as_str()).unwrap().as_ptr()
        );

        let json_str = ocaml_string(ocaml_call("compile_to_ir", ocaml_filename));
        if json_str.is_empty() {
            panic!("compilation failed")
        }

        if is_json_output_only {
            let pretty_json = serde_json::to_string_pretty(
                &serde_json::from_str::<serde_json::Value>(&json_str)
                    .expect("failed to parse JSON for pretty printing"),
            )
            .expect("failed to pretty print JSON");

            std::fs::write("test.json", format!("{}\n", pretty_json))
                .expect("failed to write test.json");

            println!("{}", pretty_json);
            return;
        }

        let collections: Vec<Collection> = serde_json::from_str(&json_str)
            .expect("failed to desearlise");

        // For now, only simulate one proc
        let proc = collections.into_iter()
            .next().expect("no collections")
            .procs.into_iter()
            .next().expect("no procs");

        let mut sim = sim::engine::Simulator::new(proc.threads);
        sim.run();

        std::process::exit(0);
    }
}
