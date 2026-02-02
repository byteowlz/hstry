fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("proto/hstry_service.proto")?;

    // Rerun build script if migrations directory changes
    println!("cargo:rerun-if-changed=migrations");

    Ok(())
}
