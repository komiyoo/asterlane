use std::ffi::OsString;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

const CONFIG_ENV: &str = "ASTERLANE_CONFIG";

#[derive(Clone, Copy, Debug)]
enum Platform {
    Linux,
    Macos,
    Windows,
    Other(&'static str),
}

pub(crate) fn resolve_config_path(explicit: Option<PathBuf>) -> io::Result<PathBuf> {
    resolve_config_path_with(
        explicit,
        std::env::var_os(CONFIG_ENV),
        current_platform(),
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
        std::env::var_os("APPDATA"),
        |path| path.try_exists(),
    )
}

fn resolve_config_path_with<F>(
    explicit: Option<PathBuf>,
    configured: Option<OsString>,
    platform: Platform,
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
    appdata: Option<OsString>,
    default_exists: F,
) -> io::Result<PathBuf>
where
    F: FnOnce(&Path) -> io::Result<bool>,
{
    if let Some(path) = explicit {
        return Ok(path);
    }
    if let Some(path) = non_blank_path(configured) {
        return Ok(path);
    }

    let path = config_path_from_root(default_config_root(
        platform,
        xdg_config_home,
        home,
        appdata,
    )?);
    match default_exists(&path) {
        Ok(true) => Ok(path),
        Ok(false) => Err(missing_default_config(&path)),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!(
                "failed to inspect default config {}: {error}",
                path.display()
            ),
        )),
    }
}

fn current_platform() -> Platform {
    match std::env::consts::OS {
        "linux" => Platform::Linux,
        "macos" => Platform::Macos,
        "windows" => Platform::Windows,
        other => Platform::Other(other),
    }
}

fn non_blank_path(value: Option<OsString>) -> Option<PathBuf> {
    value.and_then(|value| {
        if value.to_string_lossy().trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(value))
        }
    })
}

fn default_config_root(
    platform: Platform,
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
    appdata: Option<OsString>,
) -> io::Result<PathBuf> {
    match platform {
        Platform::Linux => {
            if let Some(path) = non_blank_path(xdg_config_home).filter(|path| path.is_absolute()) {
                return Ok(path);
            }
            non_blank_path(home)
                .map(|path| path.join(".config"))
                .ok_or_else(|| missing_default_root(platform))
        }
        Platform::Macos => non_blank_path(home)
            .map(|path| path.join("Library").join("Application Support"))
            .ok_or_else(|| missing_default_root(platform)),
        Platform::Windows => non_blank_path(appdata).ok_or_else(|| missing_default_root(platform)),
        Platform::Other(_) => Err(missing_default_root(platform)),
    }
}

fn config_path_from_root(root: PathBuf) -> PathBuf {
    root.join("asterlane").join("config.yaml")
}

fn missing_default_config(path: &Path) -> io::Error {
    io::Error::new(
        ErrorKind::NotFound,
        format!(
            "no config file found; pass --config PATH, set {CONFIG_ENV}, or create {}",
            path.display()
        ),
    )
}

fn missing_default_root(platform: Platform) -> io::Error {
    let detail = match platform {
        Platform::Linux => "the Linux default path \
${XDG_CONFIG_HOME:-$HOME/.config}/asterlane/config.yaml cannot be resolved because no absolute \
XDG_CONFIG_HOME or HOME is available"
            .to_string(),
        Platform::Macos => "the macOS default path \
$HOME/Library/Application Support/asterlane/config.yaml cannot be resolved because HOME is not \
available"
            .to_string(),
        Platform::Windows => "the Windows default path \
%APPDATA%\\asterlane\\config.yaml cannot be resolved because APPDATA is not available"
            .to_string(),
        Platform::Other(name) => {
            format!("platform '{name}' has no defined default config path")
        }
    };
    io::Error::new(
        ErrorKind::NotFound,
        format!(
            "no config file found; pass --config PATH, set {CONFIG_ENV}, or install the config at \
the OS default path; {detail}"
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::io;
    use std::path::{Path, PathBuf};

    use super::*;

    fn value(text: &str) -> Option<OsString> {
        Some(OsString::from(text))
    }

    fn absolute(name: &str) -> PathBuf {
        std::env::current_dir()
            .unwrap()
            .join("asterlane-config-path-tests")
            .join(name)
    }

    fn path_value(path: &Path) -> Option<OsString> {
        Some(path.as_os_str().to_owned())
    }

    #[test]
    fn flag_then_env_then_default_priority() {
        let xdg = absolute("xdg");
        let home = absolute("home");
        let flag = PathBuf::from("flag.yaml");
        let resolved = resolve_config_path_with(
            Some(flag.clone()),
            value("env.yaml"),
            Platform::Linux,
            path_value(&xdg),
            path_value(&home),
            None,
            |_| panic!("default path must not be inspected"),
        )
        .unwrap();
        assert_eq!(resolved, flag);

        let env = PathBuf::from("env.yaml");
        let resolved = resolve_config_path_with(
            None,
            Some(env.clone().into_os_string()),
            Platform::Linux,
            path_value(&xdg),
            path_value(&home),
            None,
            |_| panic!("default path must not be inspected"),
        )
        .unwrap();
        assert_eq!(resolved, env);
    }

    #[test]
    fn blank_config_env_uses_default_path() {
        let xdg = absolute("xdg");
        let expected = xdg.join("asterlane").join("config.yaml");
        let resolved = resolve_config_path_with(
            None,
            value("  \t"),
            Platform::Linux,
            path_value(&xdg),
            path_value(&absolute("home")),
            None,
            |path| {
                assert_eq!(path, expected);
                Ok(true)
            },
        )
        .unwrap();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn explicit_and_env_paths_are_returned_without_default_fallback() {
        let xdg = absolute("xdg");
        let home = absolute("home");
        let explicit = PathBuf::from("missing-explicit.yaml");
        let resolved = resolve_config_path_with(
            Some(explicit.clone()),
            None,
            Platform::Linux,
            path_value(&xdg),
            path_value(&home),
            None,
            |_| panic!("default path must not be inspected"),
        )
        .unwrap();
        assert_eq!(resolved, explicit);

        let env = PathBuf::from("missing-env.yaml");
        let resolved = resolve_config_path_with(
            None,
            Some(env.clone().into_os_string()),
            Platform::Linux,
            path_value(&xdg),
            path_value(&home),
            None,
            |_| panic!("default path must not be inspected"),
        )
        .unwrap();
        assert_eq!(resolved, env);
    }

    #[test]
    fn platform_roots_follow_linux_macos_and_windows_contracts() {
        let xdg = absolute("xdg");
        let home = absolute("home");
        assert_eq!(
            default_config_root(Platform::Linux, path_value(&xdg), path_value(&home), None,)
                .unwrap(),
            xdg
        );
        assert_eq!(
            default_config_root(Platform::Macos, None, path_value(&home), None,).unwrap(),
            home.join("Library").join("Application Support")
        );
        assert_eq!(
            default_config_root(Platform::Windows, None, None, value("windows-appdata"),).unwrap(),
            PathBuf::from("windows-appdata")
        );
    }

    #[test]
    fn linux_ignores_blank_or_relative_xdg_and_uses_home() {
        let home = absolute("home");
        for xdg in [value(""), value("relative/config")] {
            assert_eq!(
                default_config_root(Platform::Linux, xdg, path_value(&home), None,).unwrap(),
                home.join(".config")
            );
        }
    }

    #[test]
    fn config_suffix_uses_native_path_join() {
        let root = PathBuf::from("config-root");
        assert_eq!(
            config_path_from_root(root.clone()),
            root.join("asterlane").join("config.yaml")
        );
    }

    #[test]
    fn missing_default_lists_flag_env_and_computed_path() {
        let xdg = absolute("xdg");
        let expected = xdg.join("asterlane").join("config.yaml");
        let error = resolve_config_path_with(
            None,
            None,
            Platform::Linux,
            path_value(&xdg),
            None,
            None,
            |path| {
                assert_eq!(path, expected);
                Ok(false)
            },
        )
        .unwrap_err();
        let message = error.to_string();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(message.contains("--config PATH"));
        assert!(message.contains("ASTERLANE_CONFIG"));
        assert!(message.contains(&expected.display().to_string()));
    }

    #[test]
    fn missing_default_root_names_the_platform_template() {
        let error = resolve_config_path_with(
            None,
            None,
            Platform::Linux,
            value("relative/config"),
            None,
            None,
            |_path: &Path| Ok(true),
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("--config PATH"));
        assert!(message.contains("ASTERLANE_CONFIG"));
        assert!(message.contains("${XDG_CONFIG_HOME:-$HOME/.config}/asterlane/config.yaml"));
    }
}
