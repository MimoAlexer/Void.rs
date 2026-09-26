use crate::{
    Error, PROTOCOL_VERSION, Result,
    codec::{Reader, Writer},
    connection::Wire,
};
use serde::{Deserialize, Serialize};
use std::{
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAddress {
    pub host: String,
    pub port: u16,
}
impl ServerAddress {
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.is_empty()
            || input.len() > 255
            || input.chars().any(char::is_whitespace)
            || input.contains('\0')
        {
            return Err(Error::Address("expected host[:port] or [IPv6]:port".into()));
        }
        let (host, port) = if let Some(rest) = input.strip_prefix('[') {
            let (host, port) = rest
                .split_once(']')
                .ok_or_else(|| Error::Address("unclosed IPv6 bracket".into()))?;
            let port = if port.is_empty() {
                25565
            } else {
                port.strip_prefix(':')
                    .ok_or_else(|| Error::Address("invalid IPv6 port".into()))?
                    .parse()
                    .map_err(|_| Error::Address("invalid port".into()))?
            };
            (host.to_owned(), port)
        } else if input.matches(':').count() == 1 {
            let (host, port) = input.split_once(':').unwrap();
            (
                host.to_owned(),
                port.parse()
                    .map_err(|_| Error::Address("invalid port".into()))?,
            )
        } else {
            (input.to_owned(), 25565)
        };
        if host.is_empty() || port == 0 {
            return Err(Error::Address("empty hostname or port zero".into()));
        }
        Ok(Self { host, port })
    }
    pub(crate) fn connect(&self, timeout: Duration) -> Result<TcpStream> {
        let start = Instant::now();
        let mut last_error = None;
        for address in (self.host.as_str(), self.port).to_socket_addrs()? {
            let remaining = timeout.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return Err(Error::Timeout);
            }
            match TcpStream::connect_timeout(&address, remaining) {
                Ok(s) => {
                    s.set_nodelay(true)?;
                    s.set_read_timeout(Some(Duration::from_millis(25)))?;
                    s.set_write_timeout(Some(Duration::from_secs(5)))?;
                    return Ok(s);
                }
                Err(e) => last_error = Some(e),
            }
        }
        Err(last_error
            .map(Error::Io)
            .unwrap_or_else(|| Error::Address("hostname resolved to no addresses".into())))
    }
}
impl std::fmt::Display for ServerAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.host.contains(':') {
            write!(f, "[{}]:{}", self.host, self.port)
        } else {
            write!(f, "{}:{}", self.host, self.port)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStatus {
    pub version_name: String,
    pub protocol: i32,
    pub online: u32,
    pub maximum: u32,
    pub description: String,
    pub latency_ms: f64,
    pub raw: serde_json::Value,
}
pub(crate) fn handshake(address: &ServerAddress, next_state: i32) -> Writer {
    let mut w = Writer::packet(0);
    w.varint(PROTOCOL_VERSION);
    w.string(&address.host);
    w.u16(address.port);
    w.varint(next_state);
    w
}
pub fn status(address: &ServerAddress, timeout: Duration) -> Result<ServerStatus> {
    let mut wire = Wire::new(address.connect(timeout)?);
    wire.send(handshake(address, 1))?;
    wire.send(Writer::packet(0))?;
    let deadline = Instant::now() + timeout;
    let packet = wire.wait_packet(deadline)?;
    let mut r = Reader::new(&packet);
    if r.varint()? != 0 {
        return Err(Error::Malformed("expected status response"));
    }
    let raw: serde_json::Value = serde_json::from_str(&r.string(32767)?)?;
    r.finish()?;
    let mut ping = Writer::packet(1);
    let token = 0x566f69645273i64;
    ping.i64(token);
    let sent = Instant::now();
    wire.send(ping)?;
    let packet = wire.wait_packet(deadline)?;
    let mut r = Reader::new(&packet);
    if r.varint()? != 1 || r.i64()? != token {
        return Err(Error::Malformed("incorrect ping response"));
    }
    r.finish()?;
    let description = text_component(&raw["description"]);
    Ok(ServerStatus {
        version_name: raw["version"]["name"]
            .as_str()
            .unwrap_or("unknown")
            .to_owned(),
        protocol: raw["version"]["protocol"].as_i64().unwrap_or(-1) as i32,
        online: raw["players"]["online"].as_u64().unwrap_or(0) as u32,
        maximum: raw["players"]["max"].as_u64().unwrap_or(0) as u32,
        description,
        latency_ms: sent.elapsed().as_secs_f64() * 1000.0,
        raw,
    })
}
fn text_component(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) => a.iter().map(text_component).collect(),
        serde_json::Value::Object(o) => {
            let mut s = o
                .get("text")
                .or_else(|| o.get("translate"))
                .map(text_component)
                .unwrap_or_default();
            if let Some(extra) = o.get("extra") {
                s.push_str(&text_component(extra));
            }
            s
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn addresses() {
        assert_eq!(ServerAddress::parse("localhost").unwrap().port, 25565);
        assert_eq!(ServerAddress::parse("[::1]:25566").unwrap().host, "::1");
        assert!(ServerAddress::parse("foo:0").is_err());
        assert!(ServerAddress::parse("http://foo").is_err());
    }
}
