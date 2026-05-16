use std::path::PathBuf;

const BATTLEGROUP_PREFIX: &str = "funcom-seabass-";

#[derive(Debug, Clone)]
pub struct WorldProfile {
    pub battlegroup: String,
    pub namespace: String,
    pub title: Option<String>,
    pub spec_path: PathBuf,
}

/// Runtime configuration derived from ~/.dune/<bg>.yaml discovery.
/// Falls back to the known "Slackware-Arrakis" battlegroup.
#[derive(Debug, Clone)]
pub struct Config {
    pub battlegroup: String,
    pub namespace: String,
    pub title: Option<String>,
    pub world_spec: Option<PathBuf>,
    pub explicit_target: bool,
    pub scripts_dir: PathBuf,
}

impl Config {
    /// Load config by scanning ~/.dune/ for the world YAML, or fall back to
    /// compile-time defaults. `target` may be a battlegroup id or world title.
    pub fn load(target: Option<&str>) -> anyhow::Result<Self> {
        if let Some(cfg) = Self::from_dune_dir(target)? {
            return Ok(cfg);
        }
        Ok(Self::default_config())
    }

    pub fn discover_worlds() -> anyhow::Result<Vec<WorldProfile>> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/dune".into());
        let dune_dir = PathBuf::from(&home).join(".dune");
        if !dune_dir.exists() {
            return Ok(Vec::new());
        }

        let mut worlds = Vec::new();
        for entry in std::fs::read_dir(&dune_dir)? {
            let entry = entry?;
            let path = entry.path();
            let Some(fname) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !is_world_spec_filename(fname) {
                continue;
            }

            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let bg = fname.trim_end_matches(".yaml").to_string();
            worlds.push(WorldProfile {
                namespace: format!("{}{}", BATTLEGROUP_PREFIX, bg),
                battlegroup: bg,
                title: parse_title(&text),
                spec_path: path,
            });
        }

        worlds.sort_by(|a, b| a.battlegroup.cmp(&b.battlegroup));
        Ok(worlds)
    }

    fn from_dune_dir(target: Option<&str>) -> anyhow::Result<Option<Self>> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/dune".into());
        let worlds = Self::discover_worlds()?;
        let selected = match target {
            Some(target) => worlds
                .iter()
                .find(|world| world.matches(target))
                .ok_or_else(|| anyhow::anyhow!("unknown world '{}'", target))?,
            None => match worlds.first() {
                Some(world) => world,
                None => return Ok(None),
            },
        };

        Ok(Some(Self {
            namespace: selected.namespace.clone(),
            battlegroup: selected.battlegroup.clone(),
            title: selected.title.clone(),
            world_spec: Some(selected.spec_path.clone()),
            explicit_target: target.is_some(),
            scripts_dir: PathBuf::from(&home).join("dune-server/scripts"),
        }))
    }

    fn default_config() -> Self {
        let bg = "sh-db3533a2d5a25fb-xyyxbx".to_string();
        Self {
            namespace: format!("{}{}", BATTLEGROUP_PREFIX, bg),
            battlegroup: bg,
            title: Some("Slackware-Arrakis".to_string()),
            world_spec: None,
            explicit_target: false,
            scripts_dir: PathBuf::from("/home/dune/dune-server/scripts"),
        }
    }

    pub fn repo_root(&self) -> PathBuf {
        self.scripts_dir
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/home/dune/dune-server"))
    }

    pub fn user_settings_dir(&self) -> PathBuf {
        let world_dir = self.world_user_settings_dir();
        if world_dir.exists() {
            world_dir
        } else {
            self.default_user_settings_dir()
        }
    }

    pub fn settings_profile_label(&self) -> &'static str {
        if self.world_user_settings_dir().exists() {
            "profile"
        } else {
            "shared"
        }
    }

    pub fn default_user_settings_dir(&self) -> PathBuf {
        self.repo_root().join("server/scripts/setup/config")
    }

    pub fn world_user_settings_dir(&self) -> PathBuf {
        dune_home()
            .join("worlds")
            .join(&self.battlegroup)
            .join("UserSettings")
    }

    pub fn init_world_settings(&self) -> anyhow::Result<PathBuf> {
        let dst = self.world_user_settings_dir();
        std::fs::create_dir_all(&dst)?;

        let src = self.default_user_settings_dir();
        for filename in ["UserEngine.ini", "UserGame.ini"] {
            let src_file = src.join(filename);
            let dst_file = dst.join(filename);
            if dst_file.exists() {
                continue;
            }
            std::fs::copy(&src_file, &dst_file).map_err(|e| {
                anyhow::anyhow!(
                    "failed to copy {} to {}: {}",
                    src_file.display(),
                    dst_file.display(),
                    e
                )
            })?;
        }

        Ok(dst)
    }
}

impl WorldProfile {
    fn matches(&self, target: &str) -> bool {
        self.battlegroup == target || self.title.as_deref() == Some(target)
    }
}

fn is_world_spec_filename(fname: &str) -> bool {
    fname.ends_with(".yaml")
        && !fname.contains("-secret")
        && !fname.contains("-rmq")
        && !fname.contains("-fls")
        && !fname.contains("-dump-")
}

fn parse_title(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix("title:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_matches('"').to_string())
}

fn dune_home() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/dune".into());
    PathBuf::from(home).join(".dune")
}
