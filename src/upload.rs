use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use failure::{bail, ensure};
use log::*;
use serde::Serialize;

use crate::config::Authentication;

pub fn upload(
    root: &Path,
    build_path: &Option<PathBuf>,
    authentication: &Authentication,
    branch: &String,
    include_files: &Vec<PathBuf>,
    hostname: &String,
    ssl: bool,
    port: u16,
    prefix: &Option<String>,
    http_timeout: Option<u32>,
) -> Result<(), failure::Error> {
    let mut files = HashMap::new();

    for target in include_files {
        let target_dir = build_path
            .as_ref()
            .map(|p| root.join(p))
            .unwrap_or_else(|| root.into())
            .join(target);

        for entry in fs::read_dir(target_dir)? {
            let entry = entry?;
            let path = entry.path();

            if let (Some(name), Some(extension)) = (path.file_stem(), path.extension()) {
                let contents = if extension == "js" {
                    let data = {
                        let mut buf = String::new();
                        fs::File::open(&path)?.read_to_string(&mut buf)?;
                        buf
                    };
                    serde_json::Value::String(data)
                } else if extension == "wasm" {
                    let data = {
                        let mut buf = Vec::new();
                        fs::File::open(&path)?.read_to_end(&mut buf)?;
                        buf
                    };
                    let data = base64::encode(&data);
                    serde_json::json!({ "binary": data })
                } else {
                    continue;
                };

                files.insert(name.to_string_lossy().into_owned(), contents);
            }
        }
    }

    let client_builder = reqwest::blocking::Client::builder();
    let client = match http_timeout {
        None =>         client_builder.build()?,
        Some(value) =>  client_builder.timeout(
                            Duration::from_secs(value as u64)
                        ).build()?,
    };

    let url = format!(
        "{}://{}:{}/{}",
        if ssl { "https" } else { "http" },
        hostname,
        port,
        match prefix {
            Some(prefix) => format!("{}/api/user/code", prefix),
            None => "api/user/code".to_string(),
        }
    );

    #[derive(Serialize)]
    struct RequestData {
        modules: HashMap<String, serde_json::Value>,
        branch: String,
    }

    let response = authenticate(client.post(&url), authentication)
        .json(&RequestData {
            modules: files,
            branch: branch.clone(),
        })
        .send()?;

    let response_status = response.status();
    let response_url = response.url().clone();
    let response_text = response.text()?;

    ensure!(
        response_status.is_success(),
        "uploading to '{}' failed: {}",
        response_url,
        response_text,
    );

    debug!("upload finished: {}", response_text);

    let response_json: serde_json::Value = serde_json::from_str(&response_text)?;

    if let Some(s) = response_json.get("error") {
        bail!(
            "error sending to branch '{}' of '{}': {}",
            branch,
            response_url,
            s
        );
    }

    Ok(())
}

fn authenticate(
    request: reqwest::blocking::RequestBuilder,
    authentication: &Authentication,
) -> reqwest::blocking::RequestBuilder {
    match authentication {
        Authentication::Token { ref auth_token } => request.header("X-Token", auth_token.as_str()),
        Authentication::Basic {
            ref username,
            ref password,
        } => request.basic_auth(username, Some(password)),
    }
}
