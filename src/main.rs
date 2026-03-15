mod ir;
mod sim;

use std::ffi::CString;
use ir::types::*;

type Value = isize;

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
    let filename = std::env::args().nth(1).expect("Usage: anvil-sim <file.anvil>");

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

        let collections: Vec<Collection> = serde_json::from_str(&json_str)
            .expect("failed to desearlise");

        // For now, simulate the first thread of the first process
        let thread = collections.into_iter()
            .next().expect("no collections")
            .procs.into_iter()
            .next().expect("no procs")
            .threads.into_iter()
            .next().expect("no threads");

        let mut sim = sim::engine::Simulator::new(thread);
        sim.run();

        std::process::exit(0);
    }
}
