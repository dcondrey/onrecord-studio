//! `onrecord-pack-builder init-key` creates the pack publisher key in the Keychain;
//! `onrecord-pack-builder build <pack> <out-dir>` retrieves a pack and signs its manifest.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["init-key"] => onrecord_studio_lib::packs::builder::init_key()
            .map(|public_key| format!("Publisher public key: {public_key}")),
        ["build", pack, out] => tauri::async_runtime::block_on(
            onrecord_studio_lib::packs::builder::build(pack, std::path::Path::new(out)),
        )
        .map(|manifest| format!("Signed manifest: {}", manifest.display())),
        _ => Err(onrecord_studio_lib::error::AppError::DataParsing(
            "usage: onrecord-pack-builder init-key | build <pack> <out-dir>".into(),
        )),
    };
    match outcome {
        Ok(message) => println!("{message}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
