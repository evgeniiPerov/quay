use quay_core::{search, CloneFetcher, Config, RegistryFetcher, SearchFilters, SearchHit};
use std::path::Path;

pub fn run(
    query: &str,
    remote: Option<&str>,
    tag: Option<&str>,
    profile: Option<&str>,
    project: &Path,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;
    crate::commands::ensure_remotes_configured(&cfg)?;

    let f = CloneFetcher::new();
    run_with(&cfg, &f, query, remote, tag, json)
}

fn run_with<R: RegistryFetcher>(
    cfg: &Config,
    fetcher: &R,
    query: &str,
    remote: Option<&str>,
    tag: Option<&str>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let filters = SearchFilters { query, remote, tag };
    let hits = search(cfg, fetcher, &filters)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&hits)?);
    } else if hits.is_empty() {
        println!("(no results)");
    } else {
        print_hits(&hits);
    }
    Ok(())
}

fn print_hits(hits: &[SearchHit]) {
    for h in hits {
        let category = h.category.as_deref().unwrap_or("-");
        let tags = if h.tags.is_empty() {
            "-".to_string()
        } else {
            h.tags.join(",")
        };
        println!(
            "{:<24} {:<10} {:<14} {:<14} {} :: {}",
            h.name, h.version, h.remote, category, tags, h.description
        );
    }
}
