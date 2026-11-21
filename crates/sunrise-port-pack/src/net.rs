//! Port-pack's download agents and redirect checks.

pub use sunrise_proxy::{direct, env_for, host_of, keep, parse, redacted, scheme_of, set_proxy};

/// An agent for downloading `url`: through the proxy when one is set, unless `url` is a host that
/// must be reached directly.
pub fn agent(url: &str) -> ureq::Agent {
    sunrise_proxy::agent("port-pack", url, std::convert::identity)
}

/// GET from allowed HTTPS hosts, validating every redirect before contacting it.
pub(crate) fn get_from_hosts(url: &str, hosts: &[&str]) -> Result<Option<ureq::Response>, String> {
    follow(url, hosts, |address| {
        let agent = sunrise_proxy::agent("port-pack-no-redirect", address, |builder| {
            builder.redirects(0)
        });
        match agent
            .get(address)
            .set("User-Agent", crate::modrinth::AGENT)
            .call()
        {
            Ok(response) => Ok(Some(response)),
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(error) => Err(format!("{address}: {error}")),
        }
    })
}

fn follow(
    url: &str,
    hosts: &[&str],
    mut request: impl FnMut(&str) -> Result<Option<ureq::Response>, String>,
) -> Result<Option<ureq::Response>, String> {
    let mut current = checked_url(url, hosts)?;
    for _ in 0..=5 {
        let Some(response) = request(current.as_str())? else {
            return Ok(None);
        };
        if !(300..400).contains(&response.status()) {
            return Ok(Some(response));
        }
        let location = response
            .header("Location")
            .ok_or("Redirect without Location")?;
        current = checked_url(
            current.join(location).map_err(|e| e.to_string())?.as_str(),
            hosts,
        )?;
    }
    Err("Too many redirects".into())
}

fn checked_url(address: &str, hosts: &[&str]) -> Result<url::Url, String> {
    let url = url::Url::parse(address).map_err(|e| e.to_string())?;
    if url.scheme() != "https"
        || !url.host_str().is_some_and(|host| hosts.contains(&host))
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Refused download URL outside allowed HTTPS hosts".into());
    }
    Ok(url)
}

#[cfg(test)]
mod redirect_tests {
    use super::*;
    #[test]
    fn checks_each_redirect_before_requesting_it() {
        let mut seen = Vec::new();
        let result = follow("https://cdn.modrinth.com/a", &["cdn.modrinth.com"], |url| {
            seen.push(url.to_owned());
            let raw = if seen.len() == 1 {
                "HTTP/1.1 302 Found\r\nLocation: /b\r\n\r\n"
            } else {
                "HTTP/1.1 302 Found\r\nLocation: https://evil.example/steal\r\n\r\n"
            };
            Ok(Some(raw.parse().unwrap()))
        });
        assert!(result.is_err());
        assert_eq!(
            seen,
            ["https://cdn.modrinth.com/a", "https://cdn.modrinth.com/b"]
        );
        let mut seen = Vec::new();
        let result = follow("https://github.com/a", &["github.com", "release-assets.githubusercontent.com"], |url| {
            seen.push(url.to_owned());
            let raw = if seen.len() == 1 {
                "HTTP/1.1 302 Found\r\nLocation: https://release-assets.githubusercontent.com/file\r\n\r\n"
            } else {
                "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"
            };
            Ok(Some(raw.parse().unwrap()))
        }).unwrap();
        assert!(result.is_some());
        assert_eq!(seen.len(), 2);
        for url in [
            "http://cdn.modrinth.com/a",
            "https://cdn.modrinth.com.evil/a",
            "https://user@cdn.modrinth.com/a",
            "https://cdn.modrinth.com:444/a",
        ] {
            assert!(checked_url(url, &["cdn.modrinth.com"]).is_err());
        }
        let mut count = 0;
        assert!(
            follow("https://cdn.modrinth.com/a", &["cdn.modrinth.com"], |_| {
                count += 1;
                Ok(Some(
                    "HTTP/1.1 302 Found\r\nLocation: /a\r\n\r\n"
                        .parse()
                        .unwrap(),
                ))
            })
            .is_err()
        );
        assert_eq!(count, 6);
        assert!(
            follow("https://cdn.modrinth.com/a", &["cdn.modrinth.com"], |_| Ok(
                None
            ))
            .unwrap()
            .is_none()
        );
        assert!(
            follow(
                "https://cdn.modrinth.com/a",
                &["cdn.modrinth.com"],
                |_| Err("network failure".into())
            )
            .is_err()
        );
    }
}
