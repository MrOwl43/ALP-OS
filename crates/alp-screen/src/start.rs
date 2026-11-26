// SPDX-License-Identifier: AGPL-3.0-or-later
//! The first setup pages before the install choice (0.2.6 order, owner
//! 2026-11-26): Welcome (the hardware check, date, time and time zone) and
//! Network (cable or Wi-Fi). Facts from alpd's state, for the GTK pages.

use serde_json::Value;

/// One line of the hardware check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub id: String,
    pub item: String,
    /// `ok`, `note`, `warn` or `bad`.
    pub status: String,
    pub value: String,
    pub advice: String,
}

/// The hardware check lines (the network line is shown on its own) and
/// whether the check is still running.
pub fn checks(w: &Value) -> (Vec<Check>, bool) {
    let hw = &w["hardware"];
    let s = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let list = hw["checks"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|c| !matches!(c["id"].as_str(), Some("network" | "wired_network")))
                .map(|c| Check {
                    id: s(&c["id"]),
                    item: s(&c["item"]),
                    status: s(&c["status"]),
                    value: s(&c["value"]),
                    advice: s(&c["advice"]),
                })
                .collect()
        })
        .unwrap_or_default();
    (list, hw["pending"].as_bool().unwrap_or(false))
}

/// How the computer is on the network now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Net {
    Cable,
    Wifi(String),
    /// A cable is in or Wi-Fi joined, but no address from the router yet.
    Waiting,
    None {
        wifi_present: bool,
    },
}

pub fn net(w: &Value) -> Net {
    let n = &w["network"];
    let ports = n["ports"].as_array().cloned().unwrap_or_default();
    let on = |kind: &str, state: &str| {
        ports
            .iter()
            .find(|p| p["kind"].as_str() == Some(kind) && p["state"].as_str() == Some(state))
    };
    if n["online"].as_bool().unwrap_or(false) {
        if on("wired", "connected").is_some() {
            return Net::Cable;
        }
        if let Some(p) = on("wifi", "connected") {
            return Net::Wifi(p["ssid"].as_str().unwrap_or_default().to_string());
        }
        return Net::Cable;
    }
    if ports.iter().any(|p| p["state"].as_str() == Some("waiting")) {
        return Net::Waiting;
    }
    Net::None {
        wifi_present: n["wifi_present"].as_bool().unwrap_or(false),
    }
}

/// A Wi-Fi network from `wifi/networks`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiNet {
    pub ssid: String,
    /// `psk`, `open`, `8021x` or `wep`.
    pub security: String,
    /// 0 to 4.
    pub signal: u8,
    pub connected: bool,
}

pub fn networks(v: &Value) -> Vec<WifiNet> {
    let mut list: Vec<WifiNet> = v["networks"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|n| {
                    let ssid = n["ssid"].as_str()?.to_string();
                    (!ssid.is_empty()).then(|| WifiNet {
                        ssid,
                        security: n["security"].as_str().unwrap_or("psk").to_string(),
                        signal: n["signal"].as_u64().unwrap_or(0).min(4) as u8,
                        connected: n["connected"].as_bool().unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    // Strongest first; the joined one on top.
    list.sort_by(|a, b| b.connected.cmp(&a.connected).then(b.signal.cmp(&a.signal)));
    list
}

/// The Wi-Fi join job: (`running` | `done` | `failed`, message).
pub fn wifi_job(w: &Value) -> Option<(String, String)> {
    let j = &w["network"]["job"];
    let state = j["state"].as_str()?;
    let msg = j["error"]["message"]
        .as_str()
        .filter(|_| state == "failed")
        .or(j["message"].as_str())
        .unwrap_or_default();
    Some((state.to_string(), msg.to_string()))
}

/// The clock: seconds now (UTC), set from the internet, the hardware clock
/// answer (`local` | `utc`), and the chosen time zone (UTC when none).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Clock {
    pub now: u64,
    pub synced: bool,
    pub base: Option<String>,
    pub zone: Option<String>,
}

pub fn clock(w: &Value) -> Option<Clock> {
    let c = &w["clock"];
    Some(Clock {
        now: c["now"].as_u64()?,
        synced: c["synced"].as_bool().unwrap_or(false),
        base: c["base"].as_str().map(str::to_string),
        zone: w["prefs"]["time_zone"].as_str().map(str::to_string),
    })
}

/// A zone's name as shown ("America/New_York" → "America/New York").
pub fn zone_name(z: Option<&str>) -> String {
    z.map_or_else(|| "UTC".to_string(), |z| z.replace('_', " "))
}

/// The zone names from tzdata's `zone1970.tab` (third column), sorted.
pub fn zones(tab: &str) -> Vec<String> {
    let mut v: Vec<String> = tab
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split('\t').nth(2))
        .filter(|z| !z.is_empty() && z.contains('/'))
        .map(str::to_string)
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Does a zone match the search? By name (spaces and `_` alike) or by its
/// offset text such as "UTC+2".
pub fn zone_matches(zone: &str, offset: &str, q: &str) -> bool {
    let norm = |s: &str| s.to_lowercase().replace(['_', '/'], " ");
    let q = norm(q.trim());
    q.is_empty() || norm(zone).contains(&q) || offset.to_lowercase().contains(&q)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn checks_leave_out_the_network_line() {
        let w = json!({"hardware": {"checks": [
            {"id": "memory", "item": "Memory", "status": "ok", "value": "8 GB"},
            {"id": "wired_network", "item": "Network", "status": "note", "value": ""},
            {"id": "tpm", "item": "Security chip", "status": "note", "value": "none", "advice": "No chip."}
        ]}});
        let (c, pending) = checks(&w);
        assert!(!pending);
        assert_eq!(c.len(), 2);
        assert_eq!(c[1].advice, "No chip.");
        assert!(checks(&json!({"hardware": {"checks": [], "pending": true}})).1);
    }

    #[test]
    fn the_network_says_cable_wifi_waiting_or_none() {
        let port = |kind: &str, state: &str| json!({"kind": kind, "state": state, "ssid": "Home"});
        let w = |online: bool, ports: Vec<Value>| json!({"network": {"online": online, "ports": ports, "wifi_present": true}});
        assert_eq!(net(&w(true, vec![port("wired", "connected")])), Net::Cable);
        assert_eq!(
            net(&w(true, vec![port("wifi", "connected")])),
            Net::Wifi("Home".into())
        );
        assert_eq!(net(&w(false, vec![port("wired", "waiting")])), Net::Waiting);
        assert_eq!(
            net(&w(false, vec![port("wired", "off")])),
            Net::None { wifi_present: true }
        );
        assert_eq!(
            net(&json!({})),
            Net::None {
                wifi_present: false
            }
        );
    }

    #[test]
    fn networks_strongest_first_and_no_hidden_names() {
        let v = json!({"networks": [
            {"ssid": "Weak", "security": "psk", "signal": 1, "connected": false},
            {"ssid": "", "security": "psk", "signal": 4, "connected": false},
            {"ssid": "Strong", "security": "open", "signal": 4, "connected": false},
            {"ssid": "Mine", "security": "psk", "signal": 2, "connected": true}
        ]});
        let n: Vec<String> = networks(&v).into_iter().map(|n| n.ssid).collect();
        assert_eq!(n, ["Mine", "Strong", "Weak"]);
    }

    #[test]
    fn the_wifi_job_gives_the_error_when_it_failed() {
        let w = json!({"network": {"job": {"state": "failed", "message": "Connecting…",
            "error": {"message": "Wrong password."}}}});
        assert_eq!(
            wifi_job(&w),
            Some(("failed".into(), "Wrong password.".into()))
        );
        assert_eq!(wifi_job(&json!({"network": {"job": null}})), None);
    }

    #[test]
    fn zones_from_tzdata_and_search() {
        let tab = "# comment\nPT\t+3843-00908\tEurope/Lisbon\nUS\t+404251-0740023\tAmerica/New_York\tEastern\nXX\t+0\tEurope/Lisbon\n";
        assert_eq!(zones(tab), ["America/New_York", "Europe/Lisbon"]);
        assert!(zone_matches("America/New_York", "UTC-5", "new york"));
        assert!(zone_matches("Asia/Tokyo", "UTC+9", "utc+9"));
        assert!(!zone_matches("Asia/Tokyo", "UTC+9", "paris"));
        assert_eq!(zone_name(None), "UTC");
        assert_eq!(zone_name(Some("America/New_York")), "America/New York");
    }

    #[test]
    fn the_clock_and_its_zone() {
        let w = json!({"clock": {"now": 5, "synced": false, "base": "local"}, "prefs": {"time_zone": "Europe/Paris"}});
        let c = clock(&w).unwrap();
        assert_eq!(
            (c.now, c.synced, c.base.as_deref(), c.zone.as_deref()),
            (5, false, Some("local"), Some("Europe/Paris"))
        );
        assert!(clock(&json!({})).is_none());
    }
}
