//! The Ruby version of a project, read from the files that version managers
//! and Bundler write.

use std::{fs, path::Path};

use crate::cops::policy::{RubyVersion, SUPPORTED_RUBY_VERSIONS};

type Reader = fn(&str) -> Option<RubyVersion>;

const SOURCES: [(&str, Reader); 6] = [
    (".ruby-version", ruby_version_file),
    (".tool-versions", tool_versions),
    ("mise.toml", mise_toml),
    (".mise.toml", mise_toml),
    ("Gemfile.lock", lockfile),
    ("gems.locked", lockfile),
];

/// The supported Ruby version nearest to the one the project declares, or
/// `None` when no file declares one. The search starts in `start` and goes up
/// to the git root. Outside a git repository it reads `start` only.
pub(super) fn detect(start: &Path) -> Option<RubyVersion> {
    let git_root = super::find_git_root(start);
    for dir in start.ancestors() {
        let declared = SOURCES
            .iter()
            .find_map(|(name, read)| read(&fs::read_to_string(dir.join(name)).ok()?));
        if declared.is_some() {
            return declared;
        }
        if git_root.is_none() || git_root == Some(dir) {
            break;
        }
    }
    None
}

fn supported(text: &str) -> Option<RubyVersion> {
    let text = text.trim();
    let mut parts = text.strip_prefix("ruby-").unwrap_or(text).split('.');
    let major: u8 = parts.next()?.parse().ok()?;
    let minor: Option<u8> = parts.next().and_then(|part| part.parse().ok());
    let declared = match (major, minor) {
        (3, Some(5..)) => RubyVersion(4, 0),
        (major, minor) => RubyVersion(major, minor.unwrap_or(u8::MAX)),
    };
    SUPPORTED_RUBY_VERSIONS
        .into_iter()
        .rev()
        .find(|version| *version <= declared)
        .or(Some(SUPPORTED_RUBY_VERSIONS[0]))
}

fn ruby_version_file(text: &str) -> Option<RubyVersion> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))?;
    supported(line.split_whitespace().next()?)
}

fn tool_versions(text: &str) -> Option<RubyVersion> {
    text.lines().find_map(|line| {
        let mut words = line.split('#').next()?.split_whitespace();
        (words.next()? == "ruby")
            .then(|| words.next())
            .flatten()
            .and_then(supported)
    })
}

fn mise_toml(text: &str) -> Option<RubyVersion> {
    let document: toml::Value = toml::from_str(text).ok()?;
    let tool = document.get("tools")?.get("ruby")?;
    let tool = tool
        .as_array()
        .map_or(Some(tool), |versions| versions.first())?;
    supported(tool.get("version").unwrap_or(tool).as_str()?)
}

fn lockfile(text: &str) -> Option<RubyVersion> {
    let mut lines = text.lines().skip_while(|line| *line != "RUBY VERSION");
    lines.next()?;
    let mut words = lines.next()?.split_whitespace();
    (words.next()? == "ruby")
        .then(|| words.next())
        .flatten()
        .and_then(supported)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUBY_3_4: Option<RubyVersion> = Some(RubyVersion(3, 4));
    const RUBY_4_0: Option<RubyVersion> = Some(RubyVersion(4, 0));

    #[test]
    fn a_declared_version_maps_to_the_nearest_supported_version() {
        for (text, expected) in [
            ("3.4.1", RUBY_3_4),
            ("3.4", RUBY_3_4),
            ("ruby-3.4.7", RUBY_3_4),
            ("3", RUBY_3_4),
            ("3.3.6", RUBY_3_4),
            ("2.7.8", RUBY_3_4),
            ("1.9.3-p551", RUBY_3_4),
            ("4.0.9", RUBY_4_0),
            ("4.0", RUBY_4_0),
            ("ruby-4.0.0", RUBY_4_0),
            ("4", RUBY_4_0),
            ("4.1.0", RUBY_4_0),
            ("5.0.0", RUBY_4_0),
            ("3.5.0-preview1", RUBY_4_0),
            ("3.4.1p0", RUBY_3_4),
            (" 4.0.9 \n", RUBY_4_0),
            ("jruby-9.4.8.0", None),
            ("truffleruby-24.1.0", None),
            ("system", None),
            ("latest", None),
            ("ref:master", None),
            ("", None),
        ] {
            assert_eq!(supported(text), expected, "{text:?}");
        }
    }

    #[test]
    fn each_file_format_gives_its_ruby_version() {
        assert_eq!(ruby_version_file("4.0.9\n"), RUBY_4_0);
        assert_eq!(ruby_version_file("\n# pinned\nruby-3.4.1\n"), RUBY_3_4);
        assert_eq!(ruby_version_file("jruby-9.4.8.0\n"), None);
        assert_eq!(ruby_version_file(""), None);

        assert_eq!(tool_versions("nodejs 22.1.0\nruby 4.0.9\n"), RUBY_4_0);
        assert_eq!(tool_versions("ruby 3.3.6 system # old\n"), RUBY_3_4);
        assert_eq!(tool_versions("# ruby 4.0.9\nnodejs 22.1.0\n"), None);
        assert_eq!(tool_versions("rubygems 4.0.9\n"), None);

        assert_eq!(mise_toml("[tools]\nruby = \"4.0\"\n"), RUBY_4_0);
        assert_eq!(
            mise_toml("[tools]\nruby = [\"3.4.1\", \"4.0.9\"]\n"),
            RUBY_3_4
        );
        assert_eq!(
            mise_toml("[tools]\nruby = { version = \"4.0.9\" }\n"),
            RUBY_4_0
        );
        assert_eq!(
            mise_toml("[tools]\nruby = [{ version = \"4.0.9\" }]\n"),
            RUBY_4_0
        );
        assert_eq!(mise_toml("[tools]\nruby = \"latest\"\n"), None);
        assert_eq!(mise_toml("[tools]\nnode = \"22\"\n"), None);
        assert_eq!(mise_toml("[env]\nRUBY = \"4.0\"\n"), None);
        assert_eq!(mise_toml("[tools\nruby = \"4.0\"\n"), None);

        let lock = "GEM\n  specs:\n\nPLATFORMS\n  ruby\n\nRUBY VERSION\n   ruby 4.0.9p0\n\nBUNDLED WITH\n   2.7.2\n";
        assert_eq!(lockfile(lock), RUBY_4_0);
        assert_eq!(
            lockfile("RUBY VERSION\n   ruby 3.1.4p223 (jruby 9.4.5.0)\n"),
            RUBY_3_4
        );
        assert_eq!(lockfile("GEM\n  specs:\n    ruby 4.0.9\n"), None);
        assert_eq!(lockfile("RUBY VERSION\n"), None);
    }

    fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, contents) in files {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        dir
    }

    #[test]
    fn the_nearest_directory_wins_and_the_search_stops_at_the_git_root() {
        let dir = tree(&[
            (".ruby-version", "4.0.9\n"),
            ("repo/.git/HEAD", ""),
            ("repo/Gemfile.lock", "RUBY VERSION\n   ruby 3.4.1p0\n"),
            ("repo/app/models/user.rb", ""),
            ("repo/engine/.tool-versions", "ruby 4.0.9\n"),
            ("repo/engine/lib/engine.rb", ""),
            ("bare/.git/HEAD", ""),
            ("bare/lib/bare.rb", ""),
        ]);
        let at = |path: &str| detect(&dir.path().join(path));
        assert_eq!(at("repo"), RUBY_3_4);
        assert_eq!(at("repo/app/models"), RUBY_3_4);
        assert_eq!(at("repo/engine/lib"), RUBY_4_0);
        assert_eq!(at("bare/lib"), None);
    }

    #[test]
    fn outside_a_git_repository_only_the_start_directory_is_read() {
        let dir = tree(&[(".ruby-version", "4.0.9\n"), ("app/lib/a.rb", "")]);
        if super::super::find_git_root(dir.path()).is_some() {
            return;
        }
        assert_eq!(detect(dir.path()), RUBY_4_0);
        assert_eq!(detect(&dir.path().join("app")), None);
        assert_eq!(detect(&dir.path().join("app/lib")), None);
    }

    #[test]
    fn files_in_one_directory_are_read_in_a_fixed_order() {
        let ordered = [
            (".ruby-version", "4.0.9\n"),
            (".tool-versions", "ruby 4.0.9\n"),
            ("mise.toml", "[tools]\nruby = \"4.0\"\n"),
            (".mise.toml", "[tools]\nruby = \"4.0\"\n"),
            ("Gemfile.lock", "RUBY VERSION\n   ruby 4.0.9p0\n"),
            ("gems.locked", "RUBY VERSION\n   ruby 4.0.9p0\n"),
        ];
        for (index, (first, contents)) in ordered.iter().enumerate() {
            let mut files = vec![(*first, *contents)];
            if let Some((later, _)) = ordered.get(index + 1) {
                let old = match *later {
                    ".tool-versions" => "ruby 3.4.1\n",
                    "mise.toml" | ".mise.toml" => "[tools]\nruby = \"3.4\"\n",
                    _ => "RUBY VERSION\n   ruby 3.4.1p0\n",
                };
                files.push((*later, old));
            }
            let dir = tree(&files);
            assert_eq!(detect(dir.path()), RUBY_4_0, "{first}");
        }
        let unreadable = tree(&[
            (".ruby-version", "jruby-9.4.8.0\n"),
            ("Gemfile.lock", "RUBY VERSION\n   ruby 4.0.9p0\n"),
        ]);
        assert_eq!(detect(unreadable.path()), RUBY_4_0);
    }
}
