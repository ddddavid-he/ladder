/// build.rs - Compile-time mihomo binary embedding for the `bundled` feature
///
/// When built with `--features bundled`:
///   1. Detect target OS + arch from CARGO_CFG_TARGET_OS / CARGO_CFG_TARGET_ARCH
///   2. Fetch latest mihomo release tag from GitHub API
///   3. Download matching mihomo release .gz from GitHub
///   4. Decompress and write to OUT_DIR/mihomo
///   5. src/mihomo.rs uses `include_bytes!(concat!(env!("OUT_DIR"), "/mihomo"))`

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(feature = "bundled")]
    {
        download_mihomo_for_target().expect("Failed to download mihomo for bundled build");
    }
}

#[cfg(feature = "bundled")]
fn download_mihomo_for_target() -> anyhow::Result<()> {
    use flate2::read::GzDecoder;
    use std::io::Read;

    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    let mhos = match os.as_str() {
        "linux" => "linux",
        "macos" => "darwin",
        other => anyhow::bail!("Unsupported OS for bundled build: {}", other),
    };
    let mharch = match arch.as_str() {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => anyhow::bail!("Unsupported arch for bundled build: {}", other),
    };

    // Get latest version
    let version = get_latest_version()?;
    println!("cargo:warning=Bundled mihomo version: {}", version);

    let url = format!(
        "https://github.com/MetaCubeX/mihomo/releases/download/{}/mihomo-{}-{}-{}.gz",
        version, mhos, mharch, version
    );
    println!("cargo:warning=Downloading mihomo from: {}", url);

    let resp = reqwest::blocking::get(&url)
        .map_err(|e| anyhow::anyhow!("Download failed: {}", e))?;

    if !resp.status().is_success() {
        anyhow::bail!("Download HTTP error: {}", resp.status());
    }

    let gz_bytes = resp
        .bytes()
        .map_err(|e| anyhow::anyhow!("Read body failed: {}", e))?;

    let mut gz = GzDecoder::new(gz_bytes.as_ref());
    let mut buf = Vec::new();
    gz.read_to_end(&mut buf)
        .map_err(|e| anyhow::anyhow!("Decompress failed: {}", e))?;

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let dest = std::path::Path::new(&out_dir).join("mihomo");
    std::fs::write(&dest, &buf)?;

    // Set executable bit
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest, perms)?;
    }

    println!("cargo:warning=Mihomo binary written to {:?}", dest);
    Ok(())
}

#[cfg(feature = "bundled")]
fn get_latest_version() -> anyhow::Result<String> {
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
    }

    let mut req = reqwest::blocking::Client::builder()
        .user_agent("ladder-core-build/0.1")
        .build()?
        .get("https://api.github.com/repos/MetaCubeX/mihomo/releases/latest");

    // Use GITHUB_TOKEN if available (avoids anonymous rate limit in CI)
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        req = req.header("Authorization", format!("Bearer {}", token));
    }

    let resp = req
        .send()
        .map_err(|e| anyhow::anyhow!("GitHub API request failed: {}", e))?;

    let status = resp.status();
    let body = resp
        .text()
        .map_err(|e| anyhow::anyhow!("Failed to read GitHub API response body: {}", e))?;

    if !status.is_success() {
        anyhow::bail!(
            "GitHub API returned HTTP {}: {}",
            status,
            body.chars().take(300).collect::<String>()
        );
    }

    let rel: Release = serde_json::from_str(&body).map_err(|e| {
        anyhow::anyhow!(
            "JSON parse error: {} — body: {}",
            e,
            body.chars().take(300).collect::<String>()
        )
    })?;
    Ok(rel.tag_name)
}
