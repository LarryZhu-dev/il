use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TARGET: &str = "x86_64-unknown-linux-gnu";

/// The probe's native representation explicitly identifies its sole runtime call.
struct NativeProgram {
    target: &'static str,
    runtime_entry: &'static str,
}

impl NativeProgram {
    fn llvm_ir(&self) -> String {
        format!(
            "; Intelligent language native backend probe\n\
             source_filename = \"il-native-probe\"\n\
             target triple = \"{}\"\n\n\
             declare i32 @{}()\n\n\
             define i32 @main(i32 %argc, i8** %argv) {{\n\
             entry:\n\
               %status = call i32 @{}()\n\
               ret i32 %status\n\
             }}\n",
            self.target, self.runtime_entry, self.runtime_entry
        )
    }
}

fn digest(path: &Path) -> Result<String, String> {
    let data = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(data)))
}

fn run(
    root: &Path,
    report: &mut Value,
    name: &str,
    program: &str,
    arguments: &[&str],
    clean_environment: bool,
) -> Result<Output, String> {
    let mut command = Command::new(program);
    command.args(arguments).current_dir(root);
    if clean_environment {
        command.env_clear();
    }
    let output = command
        .output()
        .map_err(|error| format!("cannot execute {program}: {error}"))?;
    let mut argv = vec![program];
    argv.extend(arguments);
    report["gates"].as_array_mut().unwrap().push(json!({
        "name": name,
        "command": argv,
        "exit_code": output.status.code().unwrap_or(-1),
        "status": if output.status.success() { "PASSED" } else { "FAILED" },
        "is_test": clean_environment,
        "test_count": if clean_environment { 4 } else { 0 },
        "environment": if clean_environment { "empty" } else { "inherited" },
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
    }));
    if !output.status.success() {
        return Err(format!("{name} failed: {}", String::from_utf8_lossy(&output.stderr)));
    }
    Ok(output)
}

fn probe(root: &Path, report: &mut Value) -> Result<(), String> {
    if env::consts::OS != "linux" || env::consts::ARCH != "x86_64" {
        return Err("P01 must execute on Linux x86-64".into());
    }
    let runtime = root.join("target/debug/libil_probe_runtime.a");
    if !runtime.is_file() {
        return Err("build the il-probe-runtime static library before running the probe".into());
    }
    let program = NativeProgram { target: TARGET, runtime_entry: "il_probe_write" };
    fs::write(root.join("build/probe.ll"), program.llvm_ir()).map_err(|e| e.to_string())?;
    run(root, report, "llvm-verify", "/usr/bin/opt-14",
        &["-verify", "-disable-output", "build/probe.ll"], false)?;
    run(root, report, "llvm-object", "/usr/bin/llc-14",
        &["-filetype=obj", "-mtriple=x86_64-unknown-linux-gnu", "-relocation-model=pic",
          "build/probe.ll", "-o", "build/probe.o"], false)?;
    run(root, report, "native-link", "/usr/bin/clang-14",
        &["--target=x86_64-unknown-linux-gnu", "build/probe.o",
          "target/debug/libil_probe_runtime.a", "-ldl", "-lpthread", "-lm",
          "-Wl,--build-id=none", "-o", "build/probe.elf"], false)?;
    let binary = fs::read(root.join("build/probe.elf")).map_err(|e| e.to_string())?;
    if binary.len() < 64 || &binary[..4] != b"\x7fELF" || binary[4] != 2
        || binary[5] != 1 || u16::from_le_bytes([binary[18], binary[19]]) != 62
        || ![2, 3].contains(&u16::from_le_bytes([binary[16], binary[17]]))
    {
        return Err("linked artifact is not an executable Linux x86-64 ELF".into());
    }
    let output = run(root, report, "clean-environment-execution", "./build/probe.elf", &[], true)?;
    if output.stdout != b"probe_ok\n" || !output.stderr.is_empty() {
        report["gates"].as_array_mut().unwrap().last_mut().unwrap()["status"] = json!("FAILED");
        return Err("probe output differs from stdout=probe_ok\\n and empty stderr".into());
    }
    report["checks"] = json!([
        "linux_x86_64_elf", "exit_code_is_zero", "stdout_equals_probe_ok", "clean_environment_execution"
    ]);
    for path in ["build/probe.ll", "build/probe.o", "build/probe.elf",
                 "target/debug/libil_probe_runtime.a", "target/debug/il-native-probe"] {
        report["artifacts"].as_array_mut().unwrap().push(json!({
            "path": path, "sha256": digest(&root.join(path))?
        }));
    }
    Ok(())
}

fn main() {
    if env::args_os().len() != 1 {
        eprintln!("il-native-probe accepts no arguments");
        std::process::exit(2);
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap().to_path_buf();
    if let Err(error) = fs::create_dir_all(root.join("build")) {
        eprintln!("cannot create build directory: {error}");
        std::process::exit(1);
    }
    let mut report = json!({
        "schema_version": "1.0.0", "task_id": "P01", "target": TARGET,
        "status": "RUNNING", "gates": [], "artifacts": []
    });
    let success = match probe(&root, &mut report) {
        Ok(()) => { report["status"] = json!("PASSED"); true },
        Err(error) => {
            eprintln!("E_NATIVE_PROBE: {error}");
            report["status"] = json!("FAILED");
            report["error"] = json!(error);
            false
        }
    };
    let report_path = root.join("build/probe_gate_report.json");
    if let Err(error) = fs::write(&report_path, serde_json::to_string_pretty(&report).unwrap() + "\n") {
        eprintln!("cannot preserve native probe report: {error}");
        std::process::exit(1);
    }
    println!("{}", json!({"status": report["status"], "report": "build/probe_gate_report.json"}));
    if !success { std::process::exit(1); }
}
