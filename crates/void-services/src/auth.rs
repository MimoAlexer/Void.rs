//! Browser PKCE sign-in. A real registered application ID is a required input.
//! Tokens are intentionally not Debug/Serialize and are never put in settings.
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use url::Url;
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

const AUTHORIZE: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize";
const TOKEN: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const SCOPE: &str = "XboxLive.signin offline_access";
const CREDENTIAL_SERVICE: &str = "Void.rs/MicrosoftRefreshToken";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    pub client_id: String,
    /// Must match the public/native application registration, with loopback HTTP.
    pub redirect_uri: String,
}
impl AuthConfig {
    pub fn validate(&self) -> Result<Url> {
        ensure!(
            !self.client_id.trim().is_empty(),
            "Microsoft application client_id is required; register Void.rs first (docs/ACCOUNTS.md)"
        );
        ensure!(
            Uuid::parse_str(&self.client_id).is_ok(),
            "Microsoft application client_id must be a UUID"
        );
        let redirect = Url::parse(&self.redirect_uri)?;
        ensure!(
            redirect.scheme() == "http"
                && matches!(redirect.host_str(), Some("127.0.0.1" | "localhost"))
                && redirect.port().is_some(),
            "OAuth redirect must be an explicit localhost HTTP port"
        );
        ensure!(
            redirect.username().is_empty()
                && redirect.password().is_none()
                && redirect.query().is_none()
                && redirect.fragment().is_none(),
            "OAuth redirect must not include credentials, query or fragment"
        );
        Ok(redirect)
    }
}

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct Secret(String);
impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AccountMetadata {
    pub uuid: Uuid,
    pub username: String,
}

pub struct OnlineSession {
    pub account: AccountMetadata,
    access_token: Secret,
    pub expires_at: u64,
}
impl OnlineSession {
    pub fn access_token(&self) -> &str {
        self.access_token.expose()
    }
    pub fn expired(&self) -> bool {
        unix_time().saturating_add(60) >= self.expires_at
    }
}

#[derive(Clone, Debug)]
pub struct OfflineIdentity {
    pub username: String,
    pub uuid: Uuid,
}
pub fn offline_identity(username: &str) -> Result<OfflineIdentity> {
    ensure!(
        (3..=16).contains(&username.len())
            && username
                .bytes()
                .all(|x| x.is_ascii_alphanumeric() || x == b'_'),
        "offline username must be 3-16 ASCII letters, digits or underscores"
    );
    let hash = md5::Md5::digest(format!("OfflinePlayer:{username}").as_bytes());
    let mut bytes: [u8; 16] = hash.into();
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(OfflineIdentity {
        username: username.to_owned(),
        uuid: Uuid::from_bytes(bytes),
    })
}

pub struct AuthFlow {
    config: AuthConfig,
    verifier: Secret,
    state: Secret,
    authorize_url: Url,
    listener: tokio::net::TcpListener,
}
impl AuthFlow {
    pub fn authorization_url(&self) -> &Url {
        &self.authorize_url
    }
    pub fn open_browser(&self) -> Result<()> {
        webbrowser::open(self.authorize_url.as_str())
            .context("open system browser for Microsoft login")
    }
    /// Await a loopback callback for at most five minutes, then redeem through
    /// Xbox and Minecraft. Merely getting a Microsoft token is not login success.
    pub async fn finish(self) -> Result<(OnlineSession, Secret)> {
        let code = tokio::time::timeout(Duration::from_secs(300), self.receive_code())
            .await
            .context("Microsoft sign-in timed out")??;
        let client = client()?;
        let response = client
            .post(TOKEN)
            .form(&[
                ("client_id", self.config.client_id.as_str()),
                ("grant_type", "authorization_code"),
                ("code", code.expose()),
                ("redirect_uri", self.config.redirect_uri.as_str()),
                ("code_verifier", self.verifier.expose()),
                ("scope", SCOPE),
            ])
            .send()
            .await?;
        let token: MicrosoftToken = checked_json(response, "Microsoft token exchange").await?;
        let session = minecraft_exchange(&client, &token.access_token).await?;
        let refresh = token
            .refresh_token
            .context("Microsoft did not grant offline access")?;
        Ok((session, Secret::new(refresh)))
    }
    async fn receive_code(&self) -> Result<Secret> {
        // Ignore unrelated local HTTP traffic, but cap each connection's lifetime.
        loop {
            let (mut stream, address) = self.listener.accept().await?;
            ensure!(
                address.ip().is_loopback(),
                "unexpected non-loopback OAuth callback"
            );
            let mut bytes = Vec::new();
            let read = tokio::time::timeout(Duration::from_secs(5), async {
                let mut byte = [0; 1];
                while bytes.len() < 8192 {
                    if stream.read(&mut byte).await? == 0 {
                        break;
                    }
                    bytes.push(byte[0]);
                    if bytes.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                Ok::<(), std::io::Error>(())
            })
            .await;
            if !matches!(read, Ok(Ok(()))) {
                continue;
            }
            let request = String::from_utf8_lossy(&bytes);
            let Some(target) = request
                .lines()
                .next()
                .and_then(|line| line.strip_prefix("GET "))
                .and_then(|line| line.split_once(' '))
                .map(|(path, _)| path)
            else {
                continue;
            };
            if !target.starts_with('/') || target.starts_with("//") {
                continue;
            }
            let redirect = self.config.validate()?;
            let Ok(url) = redirect.join(target) else {
                continue;
            };
            if url.path() != redirect.path() {
                continue;
            }
            let parsed = parse_callback(&url, self.state.expose());
            let state_matches = url.query_pairs().filter(|(key, _)| key == "state").count() == 1
                && url
                    .query_pairs()
                    .any(|(key, value)| key == "state" && value == self.state.expose());
            let success = parsed.is_ok();
            let body = if success {
                "Void.rs received the sign-in response. You may return to the client."
            } else {
                "Invalid sign-in response. Return to Void.rs and start sign-in again."
            };
            let status = if success { "200 OK" } else { "400 Bad Request" };
            let reply = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes()).await;
            if success || state_matches {
                return parsed.map(Secret::new);
            }
            // Wrong-state requests cannot consume a real pending authorization.
        }
    }
}

pub async fn begin_sign_in(config: AuthConfig) -> Result<AuthFlow> {
    let redirect = config.validate()?;
    let port = redirect.port().context("OAuth port is missing")?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .context("bind OAuth callback port")?;
    let mut randomness = [0_u8; 64];
    rand::rngs::OsRng.fill_bytes(&mut randomness);
    let verifier = URL_SAFE_NO_PAD.encode(&randomness[..32]);
    let state = URL_SAFE_NO_PAD.encode(&randomness[32..]);
    randomness.zeroize();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut authorize_url = Url::parse(AUTHORIZE)?;
    authorize_url.query_pairs_mut().extend_pairs([
        ("client_id", config.client_id.as_str()),
        ("response_type", "code"),
        ("redirect_uri", config.redirect_uri.as_str()),
        ("scope", SCOPE),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("response_mode", "query"),
        ("prompt", "select_account"),
    ]);
    Ok(AuthFlow {
        config,
        verifier: Secret::new(verifier),
        state: Secret::new(state),
        authorize_url,
        listener,
    })
}

fn parse_callback(url: &Url, expected_state: &str) -> Result<String> {
    let pairs = url.query_pairs().collect::<Vec<_>>();
    let states = pairs
        .iter()
        .filter(|(key, _)| key == "state")
        .collect::<Vec<_>>();
    ensure!(
        states.len() == 1 && states[0].1 == expected_state,
        "OAuth callback state mismatch"
    );
    if pairs.iter().any(|(key, _)| key == "error") {
        bail!("Microsoft sign-in was denied or failed");
    }
    let codes = pairs
        .iter()
        .filter(|(key, _)| key == "code")
        .collect::<Vec<_>>();
    ensure!(
        codes.len() == 1 && !codes[0].1.is_empty(),
        "OAuth callback must contain exactly one code"
    );
    Ok(codes[0].1.to_string())
}

#[derive(Deserialize)]
struct MicrosoftToken {
    access_token: String,
    refresh_token: Option<String>,
}
#[derive(Deserialize)]
struct XboxToken {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    claims: XboxClaims,
}
#[derive(Deserialize)]
struct XboxClaims {
    xui: Vec<XboxUser>,
}
#[derive(Deserialize)]
struct XboxUser {
    uhs: String,
}
#[derive(Deserialize)]
struct MinecraftToken {
    access_token: String,
    expires_in: u64,
}
#[derive(Deserialize)]
struct Profile {
    id: String,
    name: String,
}

pub async fn refresh_account(
    config: &AuthConfig,
    refresh: &Secret,
) -> Result<(OnlineSession, Secret)> {
    config.validate()?;
    let client = client()?;
    let response = client
        .post(TOKEN)
        .form(&[
            ("client_id", config.client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.expose()),
            ("scope", SCOPE),
        ])
        .send()
        .await?;
    let token: MicrosoftToken = checked_json(response, "Microsoft refresh").await?;
    let session = minecraft_exchange(&client, &token.access_token).await?;
    Ok((
        session,
        Secret::new(
            token
                .refresh_token
                .unwrap_or_else(|| refresh.expose().to_owned()),
        ),
    ))
}
async fn minecraft_exchange(
    client: &reqwest::Client,
    microsoft_token: &str,
) -> Result<OnlineSession> {
    let response = client.post("https://user.auth.xboxlive.com/user/authenticate").json(&serde_json::json!({
        "Properties": { "AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("d={microsoft_token}") },
        "RelyingParty": "http://auth.xboxlive.com", "TokenType": "JWT"
    })).send().await?;
    let xbox: XboxToken = checked_json(response, "Xbox user authentication").await?;
    let response = client
        .post("https://xsts.auth.xboxlive.com/xsts/authorize")
        .json(&serde_json::json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbox.token] },
            "RelyingParty": "rp://api.minecraftservices.com/", "TokenType": "JWT"
        }))
        .send()
        .await?;
    let xsts: XboxToken = checked_json(response, "Xbox XSTS authorization (check account age/family restrictions and application registration)").await?;
    let user = xsts
        .claims
        .xui
        .first()
        .context("Xbox returned no user identity")?;
    let response = client.post("https://api.minecraftservices.com/authentication/login_with_xbox").json(&serde_json::json!({ "identityToken": format!("XBL3.0 x={};{}", user.uhs, xsts.token) })).send().await?;
    let token: MinecraftToken = checked_json(response, "Minecraft authentication").await?;
    #[derive(Deserialize)]
    struct Entitlements {
        items: Vec<serde_json::Value>,
    }
    let response = client
        .get("https://api.minecraftservices.com/entitlements/mcstore")
        .bearer_auth(&token.access_token)
        .send()
        .await?;
    let entitlements: Entitlements =
        checked_json(response, "Minecraft entitlement verification").await?;
    ensure!(
        !entitlements.items.is_empty(),
        "account has no Minecraft Java entitlement"
    );
    let response = client
        .get("https://api.minecraftservices.com/minecraft/profile")
        .bearer_auth(&token.access_token)
        .send()
        .await?;
    let profile: Profile = checked_json(response, "Minecraft Java profile").await?;
    Ok(OnlineSession {
        account: AccountMetadata {
            uuid: Uuid::parse_str(&profile.id)?,
            username: profile.name,
        },
        access_token: Secret::new(token.access_token),
        expires_at: unix_time().saturating_add(token.expires_in),
    })
}
/// These synchronous credential APIs must run on an account worker, not rendering.
pub fn save_refresh_token(account: Uuid, refresh: &Secret) -> Result<()> {
    keyring::Entry::new(CREDENTIAL_SERVICE, &account.to_string())?
        .set_password(refresh.expose())
        .context("save refresh token in operating system credential store")
}
pub fn load_refresh_token(account: Uuid) -> Result<Secret> {
    Ok(Secret::new(
        keyring::Entry::new(CREDENTIAL_SERVICE, &account.to_string())?
            .get_password()
            .context("load refresh token from operating system credential store")?,
    ))
}
pub fn forget_account(account: Uuid) -> Result<()> {
    keyring::Entry::new(CREDENTIAL_SERVICE, &account.to_string())?
        .delete_credential()
        .context("delete account credential")
}
fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .user_agent("Void.rs/0.1")
        .build()?)
}
async fn checked_json<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
    stage: &str,
) -> Result<T> {
    // Do not include remote bodies in errors: they may echo secret material.
    ensure!(
        response.status().is_success(),
        "{stage} failed with HTTP {}",
        response.status()
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= 1024 * 1024,
            "{stage} response exceeded 1 MiB"
        );
        bytes.extend_from_slice(&chunk);
    }
    let result =
        serde_json::from_slice(&bytes).with_context(|| format!("invalid {stage} response"));
    bytes.zeroize();
    result
}
fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_registration_and_remote_redirects_are_rejected() {
        assert!(
            AuthConfig {
                client_id: String::new(),
                redirect_uri: "http://127.0.0.1:43189/callback".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            AuthConfig {
                client_id: Uuid::nil().to_string(),
                redirect_uri: "https://example.com/callback".into()
            }
            .validate()
            .is_err()
        );
    }
    #[test]
    fn callback_requires_unique_matching_state_and_code() {
        assert!(
            parse_callback(
                &Url::parse("http://localhost:123/callback?state=x&code=secret").unwrap(),
                "y"
            )
            .is_err()
        );
        assert!(
            parse_callback(
                &Url::parse("http://localhost:123/callback?state=x&state=x&code=secret").unwrap(),
                "x"
            )
            .is_err()
        );
        assert_eq!(
            parse_callback(
                &Url::parse("http://localhost:123/callback?state=x&code=secret").unwrap(),
                "x"
            )
            .unwrap(),
            "secret"
        );
    }
    #[test]
    fn offline_uuid_matches_vanilla_name_uuid() {
        assert_eq!(
            offline_identity("Notch").unwrap().uuid.to_string(),
            "b50ad385-829d-3141-a216-7e7d7539ba7f"
        );
        assert!(offline_identity("invalid name").is_err());
        assert_eq!(
            format!("{:?}", Secret::new("never-print-this".into())),
            "[REDACTED]"
        );
    }
}
