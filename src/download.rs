// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use {
    anyhow::{anyhow, Context, Result},
    regex::Regex,
    reqwest::{Client, ClientBuilder},
    std::{
        cmp::Ordering,
        collections::{BTreeMap, BTreeSet},
        str::FromStr,
        time::Duration,
    },
};

const URL_MAIN: &str = "https://opensource.apple.com/";
const URL_TARBALLS: &str = "https://opensource.apple.com/tarballs";

const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:95.0) Gecko/20100101 Firefox/95.0";

/// Compare ordering of a version string.
///
/// This is super hacky and is likely wrong in many edge cases!
fn compare_version_string(a: &str, b: &str) -> Option<Ordering> {
    let a_parts = a.split('.').collect::<Vec<_>>();
    let b_parts = b.split('.').collect::<Vec<_>>();

    for (i, a_part) in a_parts.into_iter().enumerate() {
        let b_part = b_parts.get(i).unwrap_or(&"0");

        let a_int = u32::from_str(a_part);
        let b_int = u32::from_str(b_part);

        if let (Ok(a), Ok(b)) = (a_int, b_int) {
            match a.cmp(&b) {
                Ordering::Equal => continue,
                ord => {
                    return Some(ord);
                }
            }
        }
    }

    a.partial_cmp(b)
}

fn is_macos(s: &str) -> bool {
    matches!(s, "macos" | "os-x" | "mac-os-x")
}

#[derive(Clone, Debug, Eq, PartialEq, Ord)]
pub struct ReleaseRecord {
    pub entity: String,
    pub version: String,
    pub url: String,
}

impl PartialOrd for ReleaseRecord {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self.matches_entity(&other.entity) {
            compare_version_string(&self.version, &other.version)
        } else {
            match self.entity.cmp(&other.entity) {
                Ordering::Equal => compare_version_string(&self.version, &other.version),
                ord => Some(ord),
            }
        }
    }
}

impl ReleaseRecord {
    /// Whether this record belongs to the named entity.
    pub fn matches_entity(&self, s: &str) -> bool {
        s == self.entity || (is_macos(s) && is_macos(&self.entity))
    }
}

#[derive(Clone, Debug)]
pub struct ReleaseComponentRecord {
    pub entity: String,
    pub component: String,
    pub url: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord)]
pub struct ComponentRecord {
    pub component: String,
    pub filename: String,
    pub url: String,
    pub version: String,
}

impl PartialOrd for ComponentRecord {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match self.component.cmp(&other.component) {
            Ordering::Equal => compare_version_string(&self.version, &other.version),
            ord => Some(ord),
        }
    }
}

pub struct Downloader {
    client: Client,
}

impl Downloader {
    pub fn new() -> Result<Self> {
        let client = ClientBuilder::new()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(120))
            .build()
            .context("building HTTP client")?;

        Ok(Self { client })
    }

    async fn get_request(&self, url: &str) -> Result<reqwest::Response> {
        let res = self.client.get(url).send().await?;

        if res.status().is_success() {
            Ok(res)
        } else {
            Err(anyhow!("HTTP {} from {}", res.status(), url))
        }
    }

    /// Obtain records describing Apple software releases.
    pub async fn get_releases(&self) -> Result<Vec<ReleaseRecord>> {
        let res = self
            .get_request(URL_MAIN)
            .await
            .context("fetching main releases URL")?;
        let text = res.text().await?;

        let re = Regex::new(r#"<a href="(?:/release/)?(?P<entity>[^"]+)">(?P<version>[^<]+)</a>"#)?;

        let mut records = vec![];

        for caps in re.captures_iter(&text) {
            let version = &caps["version"];

            let url = format!("{}release/{}", URL_MAIN, &caps["entity"]);

            let s = caps["entity"]
                .strip_suffix(".html")
                .ok_or_else(|| anyhow!("{} does not end in .html", &caps["entity"]))?;

            // The version component is the part after the final hyphen. e.g.
            // `iphone-sdkb8` or `developer-tools-91`.
            let name = s
                .rsplit_once('-')
                .ok_or_else(|| anyhow!("{} does not contain a -", s))?
                .0;

            records.push(ReleaseRecord {
                entity: name.to_string(),
                version: version.to_string(),
                url,
            });
        }

        records.sort();

        Ok(records)
    }

    /// Obtain the software components in a given Apple software release.
    pub async fn get_release_components(
        &self,
        record: &ReleaseRecord,
    ) -> Result<Vec<ReleaseComponentRecord>> {
        let res = self
            .get_request(&record.url)
            .await
            .context("fetching release components")?;
        let text = res.text().await?;

        let re = Regex::new(r#"<a href="/tarballs/(?P<path>[^"]+)">"#)?;

        let mut records = vec![];

        for caps in re.captures_iter(&text) {
            let path = &caps["path"];

            let url = format!("{}tarballs/{}", URL_MAIN, path);

            if let Some(s) = path.strip_suffix(".tar.gz") {
                let component = s
                    .split_once('/')
                    .ok_or_else(|| anyhow!("{} does not have a /", s))?
                    .0;

                records.push(ReleaseComponentRecord {
                    entity: record.entity.clone(),
                    component: component.to_string(),
                    url,
                })
            }
        }

        Ok(records)
    }

    /// Obtain the set of named components.
    ///
    /// Values are names of Apple's open sourced components. e.g. `hfs` and `AppleFileSystemDriver`.
    pub async fn get_components(&self) -> Result<BTreeSet<String>> {
        let res = self
            .get_request(URL_TARBALLS)
            .await
            .context("fetching component tarballs URL")?;

        let text = res.text().await?;

        // One does not use regular expressions to parse HTML. Meh.
        let re = Regex::new(
            r#"<tr><td valign="top"><a href="(?P<component>[^/]+)/"><img src="/static/images/icons/folder.png""#,
        )?;

        Ok(BTreeSet::from_iter(
            re.captures_iter(&text)
                .map(|caps| caps["component"].to_string()),
        ))
    }

    /// Obtain the available versions of a component.
    ///
    /// This obtains records for each component version and doesn't fetch the archive itself.
    pub async fn get_component_versions(&self, component: &str) -> Result<Vec<ComponentRecord>> {
        let url = format!("{}/{}/", URL_TARBALLS, component);

        let res = self
            .get_request(&url)
            .await
            .context("fetching versions of component")?;
        let text = res.text().await?;

        let re = Regex::new(
            r#"<tr><td valign="top"><a href="?(?P<filename>[^">]+)"?><img src="?/static/images/icons/gz"#,
        )?;

        let mut records = vec![];

        for caps in re.captures_iter(&text) {
            let filename = caps["filename"].to_string();
            let url = format!("{}/{}/{}", URL_TARBALLS, component, filename);

            // The version is the part after the first hyphen and before the .tar.gz.

            if let Some(s) = filename.strip_suffix(".tar.gz") {
                let version = s
                    .split_once('-')
                    .ok_or_else(|| anyhow!("filename does not contain -"))?
                    .1
                    .to_string();

                records.push(ComponentRecord {
                    component: component.to_string(),
                    filename,
                    url,
                    version,
                });
            }
        }

        records.sort();

        Ok(records)
    }

    /// Obtain metadata about all versions of all components.
    pub async fn get_components_versions(&self) -> Result<BTreeMap<String, Vec<ComponentRecord>>> {
        let components = self.get_components().await.context("fetching components")?;

        let mut res = BTreeMap::new();

        for records in
            futures::future::join_all(components.iter().map(|c| self.get_component_versions(c)))
                .await
        {
            let records = records?;

            if let Some(record) = records.iter().next() {
                res.insert(record.component.clone(), records);
            }
        }

        Ok(res)
    }

    /// Get data for a given [ComponentRecord].
    ///
    /// This likely evaluates to a gzipped compressed tarball.
    pub async fn get_component_record(&self, record: &ComponentRecord) -> Result<Vec<u8>> {
        let res = self
            .get_request(&record.url)
            .await
            .context("fetching component tarball")?;

        Ok(res.bytes().await?.to_vec())
    }

    /// Obtain payload for a release component from its record.
    pub async fn get_release_component_record(
        &self,
        record: &ReleaseComponentRecord,
    ) -> Result<Vec<u8>> {
        let res = self
            .get_request(&record.url)
            .await
            .with_context(|| format!("fetching {}", record.url))?;

        Ok(res
            .bytes()
            .await
            .with_context(|| format!("reading response body from {}", record.url))?
            .to_vec())
    }
}
