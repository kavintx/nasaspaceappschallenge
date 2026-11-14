use db::{repo_sheet::SheetImportRecord, Db};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shared::{AppConfig, Error, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{info, warn};

/// NTRS `q` supports `+required -excluded` tokens but NOT quoted phrases or field
/// syntax (both silently return zero), so every query is a `+word` AND form whose
/// required words are the experiment's real name or its most distinctive terms.
pub const DEFAULT_PROGRAM_QUERIES: &[(&str, &str)] = &[
    ("BASS", "+burning +suppression +solids -II"),
    ("BASS-II", "+burning +suppression +solids +II"),
    ("SAFFIRE I", "+saffire +I -II -III -IV -V -VI"),
    ("SAFFIRE II", "+saffire +II"),
    ("SAFFIRE III", "+saffire +III"),
    ("SAME", "+smoke +aerosol +measurement +experiment"),
    ("SAME-R", "+smoke +aerosol +reflight"),
    ("FLEX", "+flame +extinguishment +droplet +experiment"),
    ("FLEX-2", "+flame +extinguishment +droplet +2"),
    ("CFI", "+cool +flames +droplet +microgravity"),
    ("ACME", "+advanced +combustion +microgravity +experiments"),
    ("SPICE", "+smoke +point +flame"),
    ("SLICE", "+SLICE +flame"),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NtrsPublication {
    pub ntrs_id: String,
    pub title: String,
    pub r#abstract: String,
    pub authors: Vec<String>,
    pub publication_date: Option<String>,
    pub doi: Option<String>,
    pub ntrs_url: String,
    pub pdf_url: Option<String>,
    pub pub_type: String,
}

#[derive(Clone)]
pub struct NtrsClient {
    http: reqwest::Client,
    base_url: String,
    pdf_dir: PathBuf,
}

const MAX_PDF_BYTES: usize = 50 * 1024 * 1024; // 50 MB limit
const MAX_SHEET_BYTES: usize = 5 * 1024 * 1024; // 5 MB CSV limit

fn is_allowed_external_host(host: &str) -> bool {
    host == "ntrs.nasa.gov"
        || host.ends_with(".nasa.gov")
        || host == "docs.google.com"
        || host == "sheets.googleapis.com"
        || host.ends_with(".googleusercontent.com")
}

fn validate_external_https_url(raw_url: &str, allow_google_sheets: bool) -> Result<reqwest::Url> {
    let parsed = reqwest::Url::parse(raw_url.trim())
        .map_err(|e| Error::BadRequest(format!("invalid URL: {e}")))?;
    if parsed.scheme() != "https" {
        return Err(Error::BadRequest("only https:// URLs are permitted".into()));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| Error::BadRequest("URL missing host".into()))?;
    let allowed = if allow_google_sheets {
        is_allowed_external_host(host)
    } else {
        host == "ntrs.nasa.gov" || host.ends_with(".nasa.gov")
    };
    if !allowed {
        return Err(Error::BadRequest(format!(
            "blocked outbound request to untrusted host: {host}"
        )));
    }
    Ok(parsed)
}

impl NtrsClient {
    pub fn new(config: &AppConfig) -> Result<Self> {
        let redirect_policy = reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 4 {
                attempt.error("too many redirects")
            } else if attempt.url().scheme() == "https"
                && attempt
                    .url()
                    .host_str()
                    .map(is_allowed_external_host)
                    .unwrap_or(false)
            {
                attempt.follow()
            } else {
                attempt.error("redirect to untrusted host blocked")
            }
        });

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(15))
            .redirect(redirect_policy)
            .user_agent("fire-intel-research-pipeline/0.1 (NASA-NTRS-Ingestor)")
            .build()?;
        Ok(Self {
            http,
            base_url: config.ntrs_base_url.trim_end_matches('/').to_string(),
            pdf_dir: PathBuf::from(&config.pdf_store_dir),
        })
    }

    pub async fn search(&self, query: &str, size: usize, page: usize) -> Result<Vec<NtrsPublication>> {
        let url = format!("{}/api/citations/search", self.base_url);
        let resp = self
            .http
            .get(&url)
            .query(&[
                ("q", query),
                ("page[size]", &size.to_string()),
                ("page[from]", &(page * size).to_string()),
            ])
            .send()
            .await?;

        let status = resp.status().as_u16();
        let body = resp.text().await?;
        if !(200..300).contains(&status) {
            return Err(Error::Pipeline(format!(
                "NTRS search HTTP {status} for query '{query}': {}",
                body.chars().take(300).collect::<String>()
            )));
        }

        let v: Value = serde_json::from_str(&body)?;
        let results = v
            .get("results")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();

        let mut pubs = Vec::with_capacity(results.len());
        for item in results {
            if let Some(p) = self.parse_citation(&item) {
                pubs.push(p);
            }
        }
        Ok(pubs)
    }

    pub async fn fetch_citation(&self, ntrs_id: &str) -> Result<NtrsPublication> {
        let clean_id = extract_ntrs_id(ntrs_id).unwrap_or_else(|| ntrs_id.trim().to_string());
        let url = format!("{}/api/citations/{}", self.base_url, clean_id);
        let resp = self.http.get(&url).send().await?;
        let status = resp.status().as_u16();
        let body = resp.text().await?;
        if !(200..300).contains(&status) {
            return Err(Error::Pipeline(format!(
                "NTRS citation {clean_id} HTTP {status}"
            )));
        }
        let v: Value = serde_json::from_str(&body)?;
        self.parse_citation(&v)
            .ok_or_else(|| Error::Pipeline(format!("unable to parse NTRS citation {clean_id}")))
    }

    fn parse_citation(&self, item: &Value) -> Option<NtrsPublication> {
        let ntrs_id = item
            .get("id")
            .and_then(|x| {
                x.as_i64()
                    .map(|n| n.to_string())
                    .or_else(|| x.as_str().map(String::from))
            })
            .filter(|s| !s.is_empty())?;

        let title = item
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        let r#abstract = item
            .get("abstract")
            .and_then(|a| a.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        let mut authors = Vec::new();
        if let Some(affs) = item.get("authorAffiliations").and_then(|a| a.as_array()) {
            for a in affs {
                if let Some(name) = a
                    .get("meta")
                    .and_then(|m| m.get("author"))
                    .and_then(|au| au.get("name"))
                    .and_then(|n| n.as_str())
                {
                    authors.push(name.trim().to_string());
                }
            }
        }

        let publication_date = item
            .get("distributionDate")
            .or_else(|| {
                item.get("publications")
                    .and_then(|p| p.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|p| p.get("publicationDate"))
            })
            .and_then(|d| d.as_str())
            .map(|s| s.split('T').next().unwrap_or(s).to_string());

        let doi = item
            .get("publications")
            .and_then(|p| p.as_array())
            .and_then(|arr| arr.first())
            .and_then(|p| p.get("doi"))
            .and_then(|d| d.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(String::from);

        let pub_type = item
            .get("stiTypeDetails")
            .or_else(|| item.get("stiType"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();

        let mut pdf_url: Option<String> = None;
        if let Some(downloads) = item.get("downloads").and_then(|d| d.as_array()) {
            for dl in downloads {
                let links = dl.get("links");
                let pdf_link = links
                    .and_then(|l| l.get("pdf"))
                    .or_else(|| links.and_then(|l| l.get("original")))
                    .and_then(|s| s.as_str());
                let mime = dl
                    .get("mimetype")
                    .and_then(|m| m.as_str())
                    .unwrap_or("");
                let name = dl.get("name").and_then(|n| n.as_str()).unwrap_or("");

                if let Some(rel) = pdf_link {
                    if mime.contains("pdf")
                        || name.to_lowercase().ends_with(".pdf")
                        || rel.to_lowercase().ends_with(".pdf")
                    {
                        pdf_url = Some(if rel.starts_with("http") {
                            rel.to_string()
                        } else {
                            format!("{}{}", self.base_url, rel)
                        });
                        break;
                    }
                }
            }
        }

        Some(NtrsPublication {
            ntrs_url: format!("{}/citations/{}", self.base_url, ntrs_id),
            ntrs_id,
            title,
            r#abstract,
            authors,
            publication_date,
            doi,
            pdf_url,
            pub_type,
        })
    }

    pub async fn download_pdf(&self, ntrs_id: &str, pdf_url: &str) -> Result<PathBuf> {
        let validated_url = validate_external_https_url(pdf_url, false)?;

        tokio::fs::create_dir_all(&self.pdf_dir).await?;
        let safe_id: String = ntrs_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if safe_id.is_empty() {
            return Err(Error::BadRequest("invalid empty NTRS ID for PDF filename".into()));
        }
        let dest = self.pdf_dir.join(format!("{safe_id}.pdf"));

        if dest.exists() {
            if let Ok(meta) = tokio::fs::metadata(&dest).await {
                if meta.len() > 1024 {
                    return Ok(dest);
                }
            }
        }

        let resp = self.http.get(validated_url).send().await?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(Error::Pipeline(format!(
                "PDF download HTTP {status} for {pdf_url}"
            )));
        }
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_PDF_BYTES {
                return Err(Error::Pdf(format!(
                    "PDF exceeds maximum allowed size ({len} bytes)"
                )));
            }
        }
        let bytes = resp.bytes().await?;
        if bytes.len() > MAX_PDF_BYTES {
            return Err(Error::Pdf("downloaded PDF exceeds 50 MB limit".into()));
        }
        if bytes.len() < 5 || &bytes[..5] != b"%PDF-" {
            return Err(Error::Pdf(format!(
                "downloaded payload from {pdf_url} is not a valid PDF ({} bytes)",
                bytes.len()
            )));
        }
        tokio::fs::write(&dest, &bytes).await?;
        info!(ntrs_id, path = %dest.display(), bytes = bytes.len(), "downloaded NTRS PDF");
        Ok(dest)
    }

    /// CLI-friendly entrypoint that permits local CSV files as well as validated HTTPS URLs.
    pub async fn import_google_sheet(&self, db: &Db, sheet_url_or_path: &str) -> Result<SheetImportRecord> {
        let trimmed = sheet_url_or_path.trim();
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            self.import_google_sheet_url(db, trimmed).await
        } else if Path::new(trimmed).exists() {
            let csv_text = tokio::fs::read_to_string(trimmed).await?;
            self.import_csv_text(db, trimmed, &csv_text).await
        } else {
            Err(Error::BadRequest(
                "sheet source must be a valid https:// Google Sheets URL or existing local CSV path".into(),
            ))
        }
    }

    /// HTTP API entrypoint: strictly enforces HTTPS + Google Sheets / NASA host allowlist (never reads local files).
    pub async fn import_google_sheet_url(&self, db: &Db, sheet_url: &str) -> Result<SheetImportRecord> {
        let export_url = normalize_google_sheet_csv_url(sheet_url);
        let validated_url = validate_external_https_url(&export_url, true)?;
        let resp = self.http.get(validated_url).send().await?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let _ = resp.text().await;
            return Err(Error::Pipeline(format!(
                "Google Sheet CSV fetch HTTP {status}"
            )));
        }
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_SHEET_BYTES {
                return Err(Error::BadRequest("Google Sheet CSV exceeds 5 MB limit".into()));
            }
        }
        let bytes = resp.bytes().await?;
        if bytes.len() > MAX_SHEET_BYTES {
            return Err(Error::BadRequest("Google Sheet CSV exceeds 5 MB limit".into()));
        }
        let csv_text = String::from_utf8_lossy(&bytes);
        self.import_csv_text(db, sheet_url, &csv_text).await
    }

    async fn import_csv_text(&self, db: &Db, source_label: &str, csv_text: &str) -> Result<SheetImportRecord> {

        let mut rdr = csv::ReaderBuilder::new()
            .flexible(true)
            .from_reader(csv_text.as_bytes());

        let headers: Vec<String> = rdr
            .headers()
            .map(|h| h.iter().map(|s| s.trim().to_lowercase()).collect())
            .unwrap_or_default();

        let mut total_rows = 0i32;
        let mut ok_rows = 0i32;
        let mut errors = Vec::new();

        for (idx, rec_res) in rdr.records().enumerate() {
            total_rows += 1;
            let rec = match rec_res {
                Ok(r) => r,
                Err(e) => {
                    errors.push(format!("row {}: CSV parse error: {e}", idx + 2));
                    continue;
                }
            };

            let mut exp_name = String::new();
            let mut ntrs_link = String::new();
            let mut abstract_text = String::new();
            let mut title_text = String::new();

            for (col_i, val) in rec.iter().enumerate() {
                let v = val.trim();
                if v.is_empty() {
                    continue;
                }
                let h = headers.get(col_i).map(String::as_str).unwrap_or("");
                if h.contains("experiment") || h.contains("program") || h.contains("acronym") {
                    exp_name = v.to_string();
                } else if h.contains("ntrs") || h.contains("link") || h.contains("url") || v.contains("ntrs.nasa.gov") {
                    ntrs_link = v.to_string();
                } else if h.contains("abstract") || h.contains("summary") {
                    abstract_text = v.to_string();
                } else if h.contains("title") {
                    title_text = v.to_string();
                }
            }

            let Some(ntrs_id) = extract_ntrs_id(&ntrs_link) else {
                if !exp_name.is_empty() {
                    let _ = db.upsert_experiment(&exp_name, &exp_name, "", "sheet").await;
                    ok_rows += 1;
                } else {
                    errors.push(format!("row {}: missing NTRS ID/link and experiment name", idx + 2));
                }
                continue;
            };

            let pub_meta = match self.fetch_citation(&ntrs_id).await {
                Ok(mut m) => {
                    if m.r#abstract.is_empty() && !abstract_text.is_empty() {
                        m.r#abstract = abstract_text.clone();
                    }
                    if m.title.is_empty() && !title_text.is_empty() {
                        m.title = title_text.clone();
                    }
                    m
                }
                Err(e) => {
                    warn!(ntrs_id, error = %e, "NTRS lookup failed during sheet import; using sheet fields");
                    NtrsPublication {
                        ntrs_id: ntrs_id.clone(),
                        title: if title_text.is_empty() {
                            format!("NTRS {ntrs_id}")
                        } else {
                            title_text
                        },
                        r#abstract: abstract_text,
                        authors: vec![],
                        publication_date: None,
                        doi: None,
                        ntrs_url: format!("{}/citations/{}", self.base_url, ntrs_id),
                        pdf_url: None,
                        pub_type: "SheetImport".to_string(),
                    }
                }
            };

            let authors_json = serde_json::to_value(&pub_meta.authors).unwrap_or_default();
            let (paper, _) = db
                .upsert_paper(
                    &pub_meta.ntrs_id,
                    &pub_meta.title,
                    &pub_meta.r#abstract,
                    &authors_json,
                    pub_meta.publication_date.as_deref(),
                    pub_meta.doi.as_deref(),
                    &pub_meta.ntrs_url,
                    pub_meta.pdf_url.as_deref(),
                    &pub_meta.pub_type,
                    "google_sheet",
                )
                .await?;

            if !exp_name.is_empty() {
                let exp = db.upsert_experiment(&exp_name, &exp_name, "", "sheet").await?;
                db.link_experiment_paper(exp.id, paper.id, "sheet", 1.0).await?;
            }

            ok_rows += 1;
        }

        db.record_sheet_import(source_label, total_rows, ok_rows, &errors)
            .await
    }

    /// Scrapes official *.nasa.gov experiment pages (or custom NASA URLs) for title/description metadata
    /// and updates the `experiments` table.
    pub async fn sync_nasa_experiment_metadata(
        &self,
        db: &Db,
        custom_urls: Option<Vec<(String, String)>>,
    ) -> Result<Vec<db::repo_records::Experiment>> {
        let targets: Vec<(String, String)> = custom_urls.unwrap_or_else(|| {
            NASA_EXPERIMENT_URLS
                .iter()
                .map(|(a, u)| (a.to_string(), u.to_string()))
                .collect()
        });

        let mut updated = Vec::new();
        for (acronym, raw_url) in targets {
            let parsed = validate_external_https_url(&raw_url, false)?;
            let mut scraped_name = String::new();
            let mut scraped_desc = String::new();

            if let Ok(resp) = self.http.get(parsed).send().await {
                if resp.status().is_success() {
                    if let Ok(html) = resp.text().await {
                        if let Some(title) = extract_html_meta(&html, "og:title")
                            .or_else(|| extract_html_tag(&html, "title"))
                        {
                            let clean = title
                                .split('|')
                                .next()
                                .unwrap_or(&title)
                                .replace("- NASA", "")
                                .trim()
                                .to_string();
                            if !clean.is_empty()
                                && !clean.eq_ignore_ascii_case("Space Station Research Explorer")
                            {
                                scraped_name = clean;
                            }
                        }
                        if let Some(desc) = extract_html_meta(&html, "description")
                            .or_else(|| extract_html_meta(&html, "og:description"))
                        {
                            let clean = desc.trim().to_string();
                            if clean.len() > 25 {
                                scraped_desc = clean;
                            }
                        }
                    }
                }
            }

            if !scraped_name.is_empty() || !scraped_desc.is_empty() {
                let row = db
                    .upsert_experiment_overwrite(&acronym, &scraped_name, &scraped_desc, "nasa_web")
                    .await?;
                updated.push(row);
            } else if let Some(existing) = db.get_experiment_by_acronym(&acronym).await? {
                updated.push(existing);
            }
        }
        Ok(updated)
    }
}

pub const NASA_EXPERIMENT_URLS: &[(&str, &str)] = &[
    ("BASS", "https://www.nasa.gov/mission/station/research-explorer/investigation/?#id=777"),
    ("BASS-II", "https://www.nasa.gov/mission/station/research-explorer/investigation/?#id=1030"),
    ("SAFFIRE I", "https://www.nasa.gov/mission/station/research-explorer/investigation/?#id=1757"),
    ("SAFFIRE II", "https://www.nasa.gov/mission/station/research-explorer/investigation/?#id=1758"),
    ("SAFFIRE III", "https://www.nasa.gov/mission/station/research-explorer/investigation/?#id=1759"),
    ("SAME", "https://www.nasa.gov/mission/station/research-explorer/investigation/?#id=165"),
    ("SAME-R", "https://www.nasa.gov/mission/station/research-explorer/investigation/?#id=737"),
    ("FLEX", "https://www.nasa.gov/mission/station/research-explorer/investigation/?#id=463"),
    ("FLEX-2", "https://www.nasa.gov/mission/station/research-explorer/investigation/?#id=841"),
    ("CFI", "https://www.nasa.gov/mission/station/research-explorer/investigation/?#id=7543"),
    ("ACME", "https://www.nasa.gov/mission/station/research-explorer/investigation/?#id=1651"),
    ("SPICE", "https://www.nasa.gov/mission/station/research-explorer/investigation/?#id=756"),
    ("SLICE", "https://www.nasa.gov/mission/station/research-explorer/investigation/?#id=850"),
];

fn extract_html_meta(html: &str, key: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let needle = key.to_lowercase();
    let idx = lower.find(&needle)?;
    let window_start = idx.saturating_sub(120);
    let window_end = (idx + 600).min(html.len());
    let snippet = &html[window_start..window_end];
    let lower_snip = snippet.to_lowercase();
    let c_idx = lower_snip.find("content=")?;
    let after = &snippet[c_idx + 8..];
    let quote = after.chars().next()?;
    if quote == '"' || quote == '\'' {
        let end = after[1..].find(quote)?;
        Some(html_unescape(&after[1..1 + end]))
    } else {
        None
    }
}

fn extract_html_tag(html: &str, tag: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let open = format!("<{}", tag.to_lowercase());
    let close = format!("</{}>", tag.to_lowercase());
    let start = lower.find(&open)?;
    let gt = lower[start..].find('>')? + start + 1;
    let end = lower[gt..].find(&close)? + gt;
    Some(html_unescape(html[gt..end].trim()))
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

pub fn normalize_google_sheet_csv_url(url: &str) -> String {
    if url.contains("export?format=csv") || url.contains("output=csv") {
        return url.to_string();
    }
    if let Some(idx) = url.find("/spreadsheets/d/") {
        let rest = &url[idx + "/spreadsheets/d/".len()..];
        let sheet_id = rest.split('/').next().unwrap_or(rest);
        let gid = url
            .split("gid=")
            .nth(1)
            .and_then(|g| g.split('&').next())
            .unwrap_or("0");
        return format!(
            "https://docs.google.com/spreadsheets/d/{sheet_id}/export?format=csv&gid={gid}"
        );
    }
    url.to_string()
}

pub fn extract_ntrs_id(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Some(trimmed.to_string());
    }
    for seg in trimmed.split(['/', '?', '&', '#']) {
        if seg.len() >= 6 && seg.chars().all(|c| c.is_ascii_digit()) {
            return Some(seg.to_string());
        }
    }
    None
}
