use std::ffi::CString;

type Value = isize;

unsafe extern "C" {
    fn caml_startup(argv: *mut *mut i8);
    fn caml_named_value(name: *const i8) -> *const Value;
    fn caml_callback_exn(closure: Value, arg: Value) -> Value;
    fn caml_callback2_exn(closure: Value, arg1: Value, arg2: Value) -> Value;
    fn caml_copy_string(s: *const i8) -> Value;
    fn caml_string_length(s: Value) -> usize;
}

fn is_exception_result(v: Value) -> bool {
    (v & 3) == 2
}

fn val_int(n: isize) -> Value {
    (n << 1) | 1
}

fn int_val(v: Value) -> isize {
    v >> 1
}

fn call1(name: &str, arg: Value) -> Value {
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

fn call2(name: &str, arg1: Value, arg2: Value) -> Value {
    unsafe {
        let cname = CString::new(name).unwrap();
        let closure = caml_named_value(cname.as_ptr());
        let result = caml_callback2_exn(*closure, arg1, arg2);
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
    let filename = std::env::args().nth(1).expect("Usage: anvil-sim <file.anvil>");

    unsafe {
        let arg0 = CString::new("anvil-sim").unwrap();
        let mut argv = [arg0.as_ptr() as *mut _, std::ptr::null_mut()];
        caml_startup(argv.as_mut_ptr());

        let ocaml_filename = caml_copy_string(
            CString::new(filename.as_str()).unwrap().as_ptr()
        );
        let coll_count = int_val(call1("compile_to_ir", ocaml_filename));
        println!("Graph collections: {}", coll_count);

        for i in 0..coll_count {
            let proc_count = int_val(call1("get_proc_count", val_int(i)));
            for j in 0..proc_count {
                let name = ocaml_string(call2("get_proc_name", val_int(i), val_int(j)));
                println!("  Collection {}, Proc {}: {}", i, j, name);
            }
        }

        std::process::exit(0);
    }
}
