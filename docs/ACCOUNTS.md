# Accounts, assets, and release trust

Void.rs includes a browser OAuth authorization-code flow with PKCE, state checking,
loopback callback, refresh, Xbox user/XSTS exchange, Minecraft token/entitlement/
profile requests, and OS credential-storage APIs. These are implemented service
APIs. End-to-end online sign-in is **unverified** until a real application is
registered and tested with a licensed account. No third-party launcher application
ID is borrowed and no successful account/session is fabricated.

## Register the client

No application has been registered for this project yet. The following setup must
be performed by the application's owner in their Microsoft account.

1. Open the [Microsoft Entra admin center](https://entra.microsoft.com/) and select
   a directory where your account can register applications. Microsoft documents
   an Azure account/tenant and at least the Application Developer role as the
   registration prerequisites. If the portal denies access, obtain access to a
   suitable tenant first. [Registration prerequisites](https://learn.microsoft.com/en-us/entra/identity-platform/quickstart-register-app)
2. Go to **Entra ID → App registrations → New registration**. Name the application
   **Void.rs** and select **Personal Microsoft accounts only**. This matches this
   client's `/consumers` authority and Minecraft's personal-account sign-in flow.
   Register it and copy **Application (client) ID** from Overview. The client ID
   is a public identifier; it is not an account password or client secret.
   [Account types and registration](https://learn.microsoft.com/en-us/entra/identity-platform/quickstart-register-app)
3. Under **Authentication**, add the **Mobile and desktop applications** platform.
   Add the custom loopback redirect **`http://localhost:43189/callback`**. In
   Advanced settings, enable **Allow public client flows**, then save. This is a
   desktop public client using a system browser and PKCE; it does not use a client
   secret. [Desktop application setup](https://learn.microsoft.com/en-us/entra/identity-platform/scenario-desktop-app-configuration),
   [redirect URI configuration](https://learn.microsoft.com/en-us/entra/identity-platform/how-to-add-redirect-uri)
4. In Void.rs **Settings → Microsoft application registration**, paste that client ID and
   set the redirect URI to **`http://localhost:43189/callback`**; select **Save
   settings**. Use the same hostname and path that you registered. The initial
   client configuration may contain `127.0.0.1`; replace it with `localhost` for
   this setup. Alternatively, edit the following values in `config.toml`:

   ```toml
   [account]
   client_id = "PASTE-YOUR-ACTUAL-APPLICATION-CLIENT-ID"
   redirect_uri = "http://localhost:43189/callback"
   ```

5. For the CLI launcher, run `void-launcher init`, open the printed `launcher.toml`
   path, and add the same values under `[microsoft]` (not `[account]`):

   ```toml
   [microsoft]
   client_id = "PASTE-YOUR-ACTUAL-APPLICATION-CLIENT-ID"
   redirect_uri = "http://localhost:43189/callback"
   ```

6. Select Microsoft sign-in in the client, or run `void-launcher login`. Sign in
   with the personal Microsoft account that owns Java Edition. Void.rs requests
   `XboxLive.signin offline_access`; do not add a client secret to configuration.
   Completion requires Microsoft, Xbox, XSTS, Minecraft entitlement, and Java
   profile checks to succeed. An application ID alone does not establish that
   this full chain is authorized or working.

If Microsoft sign-in succeeds but the Minecraft stage returns **HTTP 403**, ask
[Minecraft Support](https://help.minecraft.net/) about registering this specific
application for Minecraft Services. Provide the client ID, repository URL, and
failed stage/status; never provide tokens. Provider approval and family-account
restrictions can block authentication independently of correct OAuth code. The
current Minecraft-specific approval form could not be verified during setup, so
no automatic approval or working login is claimed. Do not substitute another
launcher's client ID.

For a redirect mismatch, compare the registered and configured hostname/path.
For a callback-port error, close the other process using port 43189 or choose a
matching free port in both configurations. An empty or invalid application ID is
rejected before opening the browser.

`begin_sign_in(config).await` binds the local port first, constructs a random state
and S256 challenge, and returns an `AuthFlow`. Open the system browser through
`flow.open_browser()`, then await `flow.finish()`. Successful completion returns an
`OnlineSession` and refresh-token `Secret` only after the Minecraft profile and
entitlement requests succeed. A pending flow expires after five minutes.

Persist public `AccountMetadata` in configuration, and put the refresh token into
the operating system credential store with `save_refresh_token`. Windows uses
Windows Credential Manager; Linux uses Secret Service. Store failure is an error,
never a reason to fall back to a plaintext file. Run credential calls on an account
worker since they are synchronous. `refresh_account` returns rotated credentials;
save the replacement after success. `forget_account` removes the stored secret.
`Secret` redacts Debug output and zeroes its owned string on drop; do not print its
explicit `expose()` result or include credentials in command arguments.

The separate `offline_identity` API calculates vanilla's MD5-based offline UUID.
It does not authenticate an account and only applies to offline-mode test servers.
Signed chat certificates and end-to-end account/server validation remain part of
the full online compatibility gate; see `STATUS.md` for current client integration.

## Assets

`AssetManager::fetch_version` resolves **26.2** in Mojang's HTTPS version manifest,
verifies the referenced metadata SHA-1, and stores it atomically.
`fetch_index` verifies index size/hash. `prepare_assets` verifies every cached or
imported object, then downloads missing objects to the native content-addressed
layout. `prepare_client_archive` obtains the hash-verified original JAR for model,
texture, and data reading; it does not execute Java. Pass an existing `.minecraft`
directory to reuse its verified bytes. Preserve source resource formats.

Downloads are bounded, reject unexpected lengths/hashes, and publish only complete
files through a same-directory temporary file. Run imports/hash checks on a worker.
Network tests are distinct from deterministic unit tests. Do not commit downloaded
Mojang data, tokens, or client/server JARs.

## Signed updates

The updater accepts a JSON envelope containing a base64 `payload` (exact signed
UTF-8 JSON bytes) and base64 Ed25519 `signature`. `verify_manifest` requires the
release public key from the trusted launcher, expected channel/platform, and
installed version. Payload schema 1 has `version`, `channel`, `platform`, `artifact`
(one filename), HTTPS `url`, lowercase `sha256`, and exact `size`. Downgrades,
wrong channels/platforms, stable prereleases, path escapes, and invalid signatures
are rejected. Release private keys must stay outside the repository.

`stage_update` verifies the executable and publishes a complete sibling directory.
`activate_staged(root, &verified_release)` rechecks bytes against the signature-
verified manifest, backs up `current` as `previous`, and restores it if activation
fails. Stop the running client first. `rollback` preserves the failed version and
restores the previous one. Settings and mods belong outside these directories.
Existing backups are never silently deleted. Only single executable artifacts are
implemented; bundled runtime/assets packaging and automatic boot-health recovery
require launcher integration and release qualification.

Sources: [Microsoft authorization-code and PKCE documentation](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow),
[Mojang version manifest](https://piston-meta.mojang.com/mc/game/version_manifest_v2.json),
[OS credential backend features](https://docs.rs/crate/keyring/3.6.1/features).
