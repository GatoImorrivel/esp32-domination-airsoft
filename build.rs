use std::process::Command;

fn main() {
    embuild::espidf::sysenv::output();
    // Make sure we re-run build.rs if anything in web-ui/ changes
    println!("cargo:rerun-if-changed=web-ui/package.json");
    println!("cargo:rerun-if-changed=web-ui/package-lock.json");
    println!("cargo:rerun-if-changed=web-ui/src");

    // 1️⃣ Run `npm install` in web-ui/
    let status = Command::new("npm")
        .arg("install")
        .current_dir("web-ui")
        .status()
        .expect("Failed to run npm install");
    if !status.success() {
        panic!("npm install failed");
    }

    // 2️⃣ Run `npm run build` in web-ui/
    let status = Command::new("npm")
        .arg("run")
        .arg("build")
        .current_dir("web-ui")
        .status()
        .expect("Failed to run npm run build");
    if !status.success() {
        panic!("npm run build failed");
    }
}
