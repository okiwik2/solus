fn main() {
    println!("cargo:rerun-if-changed=res/solus.rc");
    println!("cargo:rerun-if-changed=res/solus.manifest");
    println!("cargo:rerun-if-changed=res/solus.ico");
    embed_resource::compile("res/solus.rc", embed_resource::NONE)
        .manifest_required()
        .unwrap();
}
