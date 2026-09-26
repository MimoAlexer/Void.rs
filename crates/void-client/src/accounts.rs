//! Only public profile metadata belongs in this file. Tokens use the OS keyring.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};
use void_services::auth::AccountMetadata;

#[derive(Default, Deserialize, Serialize)]
struct Accounts {
    #[serde(default)]
    accounts: Vec<AccountMetadata>,
}
pub fn read(config: &Path) -> Result<Vec<AccountMetadata>> {
    let path = config.with_file_name("accounts.toml");
    if !path.exists() {
        return Ok(Vec::new());
    }
    Ok(toml::from_str::<Accounts>(&std::fs::read_to_string(path)?)?.accounts)
}
pub fn save(config: &Path, profile: AccountMetadata) -> Result<Vec<AccountMetadata>> {
    let mut accounts = read(config)?;
    accounts.retain(|a| a.uuid != profile.uuid);
    accounts.push(profile);
    let path = config.with_file_name("accounts.toml");
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let bytes = toml::to_string_pretty(&Accounts {
        accounts: accounts.clone(),
    })?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(accounts)
}
