fn main() {
    println!("cargo:rerun-if-changed=resources/app.rc");
    println!("cargo:rerun-if-changed=resources/app.manifest");
    embed_resource::compile("resources/app.rc", embed_resource::NONE)
        .manifest_required()
        .expect("embed the Windows application manifest");
}
