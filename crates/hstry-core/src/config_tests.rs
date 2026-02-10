//! Unit tests for configuration.

#[cfg(test)]
mod path_expansion_tests {
    use super::super::Config;
    use std::path::PathBuf;

    #[test]
    fn expand_path_handles_tilde() {
        let result = Config::expand_path("~/test");
        // Should not start with ~ after expansion
        assert!(!result.to_string_lossy().starts_with('~'));
    }

    #[test]
    fn expand_path_handles_absolute_path() {
        let result = Config::expand_path("/absolute/path");
        assert_eq!(result, PathBuf::from("/absolute/path"));
    }

    #[test]
    fn expand_path_handles_relative_path() {
        let result = Config::expand_path("relative/path");
        assert_eq!(result, PathBuf::from("relative/path"));
    }

    #[test]
    fn expand_path_handles_env_vars() {
        temp_env::with_var("HSTRY_TEST_VAR", Some("/test/path"), || {
            let result = Config::expand_path("$HSTRY_TEST_VAR/subdir");
            assert!(result.to_string_lossy().contains("/test/path"));
        });
    }
}

#[cfg(test)]
mod default_config_tests {
    use super::super::Config;

    #[test]
    fn default_has_database_path() {
        let config = Config::default();
        assert!(config.database.to_string_lossy().contains("hstry"));
        assert!(config.database.to_string_lossy().ends_with(".db"));
    }

    #[test]
    fn default_has_adapter_paths() {
        let config = Config::default();
        assert!(!config.adapter_paths.is_empty());
    }

    #[test]
    fn default_has_official_adapter_repo() {
        let config = Config::default();
        assert!(!config.adapter_repos.is_empty());
        assert!(config.adapter_repos.iter().any(|r| r.name == "official"));
    }

    #[test]
    fn default_js_runtime_is_auto() {
        let config = Config::default();
        assert_eq!(config.js_runtime, "auto");
    }

    #[test]
    fn default_service_disabled() {
        let config = Config::default();
        assert!(!config.service.enabled);
    }

    #[test]
    fn default_search_api_enabled() {
        let config = Config::default();
        assert!(config.service.search_api);
    }
}

#[cfg(test)]
mod adapter_enabled_tests {
    use super::super::{AdapterConfig, Config};

    #[test]
    fn adapter_enabled_returns_true_by_default() {
        let config = Config::default();
        assert!(config.adapter_enabled("nonexistent"));
        assert!(config.adapter_enabled("opencode"));
    }

    #[test]
    fn adapter_enabled_respects_config() {
        let mut config = Config::default();
        config.adapters.push(AdapterConfig {
            name: "disabled-adapter".to_string(),
            enabled: false,
        });
        config.adapters.push(AdapterConfig {
            name: "enabled-adapter".to_string(),
            enabled: true,
        });

        assert!(!config.adapter_enabled("disabled-adapter"));
        assert!(config.adapter_enabled("enabled-adapter"));
        assert!(config.adapter_enabled("other-adapter")); // Not in config, default true
    }
}

#[cfg(test)]
mod search_index_path_tests {
    use super::super::{Config, SearchConfig};
    use std::path::PathBuf;

    #[test]
    fn uses_explicit_path_when_set() {
        let mut config = Config::default();
        config.search = SearchConfig {
            index_path: Some(PathBuf::from("/custom/index")),
            index_batch_size: 500,
        };
        assert_eq!(config.search_index_path(), PathBuf::from("/custom/index"));
    }

    #[test]
    fn derives_from_database_path_when_not_set() {
        let mut config = Config::default();
        config.database = PathBuf::from("/data/hstry/hstry.db");
        config.search.index_path = None;

        let index_path = config.search_index_path();
        assert!(index_path.to_string_lossy().contains("tantivy"));
        assert!(index_path.to_string_lossy().contains("index"));
    }
}

#[cfg(test)]
mod config_serialization_tests {
    use super::super::Config;
    use std::path::PathBuf;

    #[test]
    fn toml_roundtrip() {
        let mut config = Config::default();
        config.database = PathBuf::from("/test/db.db");
        config.js_runtime = "bun".to_string();
        config.workspaces = vec!["~/projects".to_string()];

        let toml_str = toml::to_string(&config).expect("serialize");
        let parsed: Config = toml::from_str(&toml_str).expect("deserialize");

        assert_eq!(parsed.database, config.database);
        assert_eq!(parsed.js_runtime, config.js_runtime);
        assert_eq!(parsed.workspaces, config.workspaces);
    }
}

#[cfg(test)]
mod service_config_tests {
    use super::super::ServiceConfig;

    #[test]
    fn default_poll_interval() {
        let config = ServiceConfig::default();
        assert_eq!(config.poll_interval_secs, 30);
    }

    #[test]
    fn default_search_api_enabled() {
        let config = ServiceConfig::default();
        assert!(config.search_api);
    }

    #[test]
    fn default_search_port_none() {
        let config = ServiceConfig::default();
        assert!(config.search_port.is_none());
    }
}

#[cfg(test)]
mod adapter_repo_source_tests {
    use super::super::AdapterRepoSource;

    #[test]
    fn git_adapters_path() {
        let source = AdapterRepoSource::Git {
            url: "https://github.com/test/repo".to_string(),
            git_ref: "main".to_string(),
            path: "custom/adapters".to_string(),
        };
        assert_eq!(source.adapters_path(), "custom/adapters");
    }

    #[test]
    fn archive_adapters_path() {
        let source = AdapterRepoSource::Archive {
            url: "https://example.com/adapters.tar.gz".to_string(),
            path: "adapters".to_string(),
        };
        assert_eq!(source.adapters_path(), "adapters");
    }

    #[test]
    fn local_adapters_path() {
        let source = AdapterRepoSource::Local {
            path: "/local/adapters".to_string(),
        };
        assert_eq!(source.adapters_path(), "/local/adapters");
    }
}

#[cfg(test)]
mod remote_config_tests {
    use super::super::RemoteConfig;

    #[test]
    fn serde_roundtrip() {
        let remote = RemoteConfig {
            name: "laptop".to_string(),
            host: "user@laptop.local".to_string(),
            database_path: Some("/custom/path.db".to_string()),
            port: Some(2222),
            identity_file: Some("~/.ssh/id_ed25519".to_string()),
            enabled: true,
        };

        let json = serde_json::to_string(&remote).expect("serialize");
        let parsed: RemoteConfig = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.name, remote.name);
        assert_eq!(parsed.host, remote.host);
        assert_eq!(parsed.port, remote.port);
    }
}
