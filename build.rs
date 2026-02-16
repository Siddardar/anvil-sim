  use std::path::PathBuf;                                   
  use std::process::Command;                                                                                                
                                                                                                                            
  fn main() {                                                                                                               
      let anvil_dir = PathBuf::from("../anvil");

      let status = Command::new("dune")
          .arg("build")
          .current_dir(&anvil_dir)
          .status()
          .expect("Failed to run dune build");
      assert!(status.success(), "Dune build failed");

      // Link the complete object (has everything: anvil + stdlib + runtime)
      let ffi_obj = anvil_dir.join("_build/default/lib/anvil_ffi.o");
      println!("cargo:rustc-link-arg={}", ffi_obj.display());

      // Rerun if compiler changes
      println!("cargo:rerun-if-changed=../anvil/lib/");
  }