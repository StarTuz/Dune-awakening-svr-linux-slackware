use std::path::PathBuf;

/// Runtime configuration derived from ~/.dune/<bg>.yaml discovery.
/// Falls back to the known "Slackware-Arrakis" battlegroup.
#[derive(Debug, Clone)]
pub struct Config {
    pub battlegroup: String,
    pub namespace: String,
    pub scripts_dir: PathBuf,
}

impl Config {
    /// Load config by scanning ~/.dune/ for the world YAML, or fall back to
    /// compile-time defaults.
    pub fn load() -> anyhow::Result<Self> {
        if let Some(cfg) = Self::from_dune_dir()? {
            return Ok(cfg);
        }
        Ok(Self::default_config())
    }

    fn from_dune_dir() -> anyhow::Result<Option<Self>> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/dune".into());
        let dune_dir = PathBuf::from(&home).join(".dune");
        if !dune_dir.exists() {
            return Ok(None);
        }
        for entry in std::fs::read_dir(&dune_dir)? {
            let entry = entry?;
            let fname = entry.file_name();
            let fname = fname.to_string_lossy();
            // Match <bg>.yaml; skip secrets and rmq files
            if fname.ends_with(".yaml")
                && !fname.contains("-secret")
                && !fname.contains("-rmq")
                && !fname.contains("-fls")
            {
                let bg = fname.trim_end_matches(".yaml").to_string();
                return Ok(Some(Self {
                    namespace: format!("funcom-seabass-{}", bg),
                    battlegroup: bg,
                    scripts_dir: PathBuf::from(&home).join("dune-server/scripts"),
                }));
            }
        }
        Ok(None)
    }

    fn default_config() -> Self {
        let bg = "sh-db3533a2d5a25fb-xyyxbx".to_string();
        Self {
            namespace: format!("funcom-seabass-{}", bg),
            battlegroup: bg,
            scripts_dir: PathBuf::from("/home/dune/dune-server/scripts"),
        }
    }
}
