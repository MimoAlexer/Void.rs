# Rust launcher

Build `cargo build --release -p void-launcher -p void-client`. Keep both executables
in the same directory for a portable development install. Running `void-launcher`
without arguments starts `install_root/current/void-client[.exe]`, or the sibling
client executable when no release is installed. It exits after the child acquires
its lifetime install lock. Launching never contacts an update service.

```powershell
void-launcher init
void-launcher --config path/to/launcher.toml assets --minecraft-dir "$env:APPDATA/.minecraft"
void-launcher --config path/to/launcher.toml assets --metadata-only
void-launcher login
void-launcher accounts
void-launcher refresh ACCOUNT-UUID
void-launcher forget ACCOUNT-UUID
void-launcher launch -- --config path/to/client.toml
void-launcher install-update https://publisher.example/signed-release.json
void-launcher rollback
```

The global `--config` option selects a TOML file. Its default is the platform
configuration directory returned by `directories::ProjectDirs("rs", "Void",
"Void.rs")`, typically `%APPDATA%/Void/Void.rs/config/launcher.toml` on Windows or
`~/.config/void.rs/launcher.toml` on Linux. `init` prints the actual path and refuses to
replace existing configuration. Relative paths resolve against that file's parent.

```toml
schema = 1
channel = "stable" # or "preview"
install_root = "install"
asset_cache = "cache"
# client_path = "path/to/a/development/void-client.exe"
# release_public_key = "ACTUAL_PUBLISHER_ED25519_PUBLIC_KEY_AS_64_HEX_CHARACTERS"

# Set this only after registering your own public/native Microsoft application.
# [microsoft]
# client_id = "YOUR-REGISTERED-APPLICATION-ID"
# redirect_uri = "http://localhost:43189/callback"
```

`client_path` explicitly overrides installed/sibling selection. Arguments after
`launch --` are forwarded as individual arguments, without a shell. No login token
is accepted through CLI arguments. Login opens the browser, validates the provider
chain, saves only the refresh token in the OS credential store, and stores public
account metadata in `accounts.toml` next to launcher configuration. Account
refresh, selection and credential retrieval are services the client UI can reuse.

## Update trust and recovery

There is no built-in release key or signed release URL yet. Pin the actual
publisher key in configuration before installing a signed release. Keys supplied
by an update response are never trusted. `install-update` requires an HTTPS URL and
verifies an Ed25519-signed manifest, exact channel/platform, increasing semantic
version, expected executable filename, exact size and SHA-256 before activation.
See `ACCOUNTS.md` for the manifest wire format. A signed manifest is preserved in
the installed directory. Signature verification is implemented and unit-tested;
an actual publisher-signed release remains an external input.

Installed clients retain a shared OS file lock. The launcher holds the same lock
through process startup until the client acknowledges acquiring its own, avoiding
a gap during which an updater could replace a running client. Update and rollback
commands require an exclusive lock and fail while a compatible client is running.
Direct launches of a client inside `current` infer the same lock. Development
clients launched outside this installation are independent of installed releases.

Activation retains `previous`, restores it on a rename failure, and does not touch
settings or mods. `rollback` retains the failed version and restores `previous`.
Existing `previous`, `staged`, or `failed` directories are not silently deleted;
archive them intentionally before repeating an operation that would replace one.
Downloads publish completed stages only. Automatic update discovery, automatic
post-crash rollback, launcher self-update, release-key rotation, and multi-file
release bundles remain future work.

## Developer verification

The `void-services` example checks actual support without requiring login:

```powershell
cargo run -p void-services --example service_probe -- assets .local/service-cache
cargo run -p void-services --example service_probe -- wasm PATH_TO_BUILT_MOD_FOLDER
cargo run -p void-services --example service_probe -- trusted-native PATH_TO_BUILT_MOD_FOLDER
```

Only use the trusted-native probe for a library you trust, such as the in-repository
example you built. Native loading executes unrestricted process code.
