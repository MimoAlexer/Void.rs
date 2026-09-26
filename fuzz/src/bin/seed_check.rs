use std::{fs, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("seeds");
    let mut files = 0;
    let mut mutations = 0;
    for (name, run) in [
        ("frame_codec", void_protocol_fuzz::frame_codec as fn(&[u8])),
        ("network_nbt", void_protocol_fuzz::network_nbt as fn(&[u8])),
    ] {
        let mut entries = fs::read_dir(root.join(name))?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if !entry.file_type()?.is_file() {
                continue;
            }
            let bytes = fs::read(entry.path())?;
            if bytes.len() > void_protocol_fuzz::MAX_INPUT {
                return Err(format!("seed exceeds input limit: {}", entry.path().display()).into());
            }
            run(&bytes);
            files += 1;
            for index in 0..bytes.len() {
                let mut changed = bytes.clone();
                changed[index] ^= 0x80;
                run(&changed);
                mutations += 1;
            }
        }
    }
    println!(
        "Checked {files} seed files and {mutations} deterministic bit mutations. This is not a coverage-guided fuzz campaign."
    );
    Ok(())
}
