//! Target-file discovery: `.rb` files, `.gitignore`-aware, with excludes.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use globset::GlobMatcher;
use ignore::WalkBuilder;

use crate::config::matches_any;

/// Collect the `.rb` files to inspect.
///
/// Explicit file paths are kept whatever their name or extension (`.rb` files only, plus files named explicitly, as RuboCop lints files you
/// name). Directories are walked `.gitignore`-aware, skipping hidden files,
/// and yield only files with the `.rb` extension: extensionless files such as
/// `LICENSE` or `bin/` scripts are skipped. Excludes apply to both.
pub fn collect_files(
    paths: &[PathBuf],
    config_root: &Path,
    excludes: &[GlobMatcher],
) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for path in paths {
        if path.is_file() {
            push_candidate(path, config_root, excludes, &mut out);
        } else if path.is_dir() {
            let walker = WalkBuilder::new(path).build();
            for entry in walker.flatten() {
                if entry.file_type().is_some_and(|ft| ft.is_file()) && is_ruby_file(entry.path()) {
                    push_candidate(entry.path(), config_root, excludes, &mut out);
                }
            }
        } else {
            bail!("path `{}` does not exist", path.display());
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Whether a file found by a directory walk is Ruby: it has exactly the `.rb`
/// extension (case-sensitive).
fn is_ruby_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "rb")
}

/// Keep a file unless it is excluded. Exclude globs are matched against the
/// path relative to the config root; the path itself is kept as-is for
/// reading.
fn push_candidate(
    path: &Path,
    config_root: &Path,
    excludes: &[GlobMatcher],
    out: &mut Vec<PathBuf>,
) {
    let relative = display_path(path, config_root);
    if matches_any(excludes, &relative) {
        return;
    }
    out.push(path.to_path_buf());
}

/// The path to report for a file: relative to the config root when possible,
/// without a leading `./`. Only the display name changes, not the disk path;
/// interior parent components are preserved rather than resolving through symlinks.
/// Leading `..` in a relative input walks the actual cwd before rebasing it.
pub fn display_path(path: &Path, config_root: &Path) -> PathBuf {
    let absolute;
    let relative = if path.is_relative() && config_root.is_absolute() {
        let mut base = std::env::current_dir().unwrap_or_default();
        let mut components = path.components();
        // getcwd names the actual working directory, so leading parent steps
        // can be consumed safely. Do not collapse `link/..` inside the input:
        // it may traverse a symlink and must retain its alias-specific policy.
        while let Some(component) = components.clone().next() {
            match component {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    base.pop();
                }
                _ => break,
            }
            components.next();
        }
        absolute = base.join(components.as_path());
        absolute.strip_prefix(config_root).unwrap_or(path)
    } else {
        path.strip_prefix(config_root).unwrap_or(path)
    };
    // Components remove interior `.` too, without resolving `..` or symlinks.
    relative
        .strip_prefix(".")
        .unwrap_or(relative)
        .components()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::compile_globs;

    fn setup(tree: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        for (path, contents) in tree {
            let full = tmp.path().join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, contents).unwrap();
        }
        let root = tmp.path().to_path_buf();
        (tmp, root)
    }

    fn assert_files(actual: &[PathBuf], expected: &[&str]) {
        let actual: Vec<String> = actual.iter().map(|p| p.display().to_string()).collect();
        assert_eq!(actual, expected.to_vec());
    }

    #[test]
    fn display_paths_strip_leading_current_directory_components() {
        for root in [Path::new(""), Path::new("/project")] {
            for (path, expected) in [
                ("./ok.rb", "ok.rb"),
                ("././ok.rb", "ok.rb"),
                ("./lib/nested.rb", "lib/nested.rb"),
                ("./../ok.rb", "../ok.rb"),
                (".hidden.rb", ".hidden.rb"),
            ] {
                assert_eq!(
                    display_path(Path::new(path), root).to_str().unwrap(),
                    expected,
                    "path={path}, root={root:?}"
                );
            }
        }
    }

    #[test]
    fn display_paths_rebase_leading_parent_steps_from_the_actual_cwd() {
        let cwd = std::env::current_dir().unwrap();
        let input = Path::new("..")
            .join(cwd.file_name().unwrap())
            .join("lib/a.rb");
        assert_eq!(display_path(&input, &cwd), Path::new("lib/a.rb"));
        assert_eq!(
            display_path(Path::new("lib/link/../a.rb"), &cwd),
            Path::new("lib/link/../a.rb")
        );
    }

    #[test]
    fn display_paths_keep_config_relative_and_outside_paths() {
        for (path, expected) in [
            ("/project/lib/ok.rb", "lib/ok.rb"),
            ("lib/ok.rb", "lib/ok.rb"),
            ("../ok.rb", "../ok.rb"),
            ("/outside/ok.rb", "/outside/ok.rb"),
            ("lib/link/../ok.rb", "lib/link/../ok.rb"),
        ] {
            assert_eq!(
                display_path(Path::new(path), Path::new("/project"))
                    .to_str()
                    .unwrap(),
                expected,
                "{path}"
            );
        }
    }

    #[test]
    fn collects_rb_files_recursively() {
        let (_tmp, root) = setup(&[
            ("app/models/user.rb", "class User; end"),
            ("lib/deep/thing.rb", "1 + 1"),
            ("app/assets/style.css", "body {}"),
            ("notes.txt", "not ruby"),
        ]);
        let files = collect_files(std::slice::from_ref(&root), &root, &[]).unwrap();
        // Paths stay absolute (as read from disk); display is relativized.
        assert!(files.iter().all(|p| p.is_absolute()));
        assert_files(
            &files
                .iter()
                .map(|p| display_path(p, &root))
                .collect::<Vec<_>>(),
            &["app/models/user.rb", "lib/deep/thing.rb"],
        );
    }

    #[test]
    fn directory_walk_skips_extensionless_files() {
        let (_tmp, root) = setup(&[
            ("LICENSE", "MIT License\n\nCopyright (c) 2026 Someone\n"),
            ("bin/console", "#!/usr/bin/env ruby\nrequire 'irb'\n"),
            ("lib/irb/lc/help-message", "Usage: irb.rb [options]\n"),
            ("lib/gem.rb", "module Gem; end\n"),
        ]);
        let files = collect_files(std::slice::from_ref(&root), &root, &[]).unwrap();
        assert_files(
            &files
                .iter()
                .map(|p| display_path(p, &root))
                .collect::<Vec<_>>(),
            &["lib/gem.rb"],
        );
    }

    #[test]
    fn directory_walk_requires_exact_rb_extension() {
        let (_tmp, root) = setup(&[
            ("Rakefile", "task :default"),
            ("lib/tasks/build.rake", "task :build"),
            ("gem.gemspec", "Gem::Specification.new"),
            ("lib/upper.RB", "1"),
            ("lib/backup.rb.orig", "1"),
            ("lib/x.rb", "1"),
        ]);
        let files = collect_files(std::slice::from_ref(&root), &root, &[]).unwrap();
        assert_files(
            &files
                .iter()
                .map(|p| display_path(p, &root))
                .collect::<Vec<_>>(),
            &["lib/x.rb"],
        );
    }

    #[test]
    fn applies_global_excludes() {
        let (_tmp, root) = setup(&[
            ("db/schema.rb", "create_table"),
            ("app/models/user.rb", "class User; end"),
            ("vendor/gem/lib/x.rb", "1"),
        ]);
        let mut excludes = crate::config::builtin_excludes().unwrap();
        excludes.extend(compile_globs(&["vendor/**".to_string()], "test").unwrap());
        let files = collect_files(std::slice::from_ref(&root), &root, &excludes).unwrap();
        assert_files(
            &files
                .iter()
                .map(|p| display_path(p, &root))
                .collect::<Vec<_>>(),
            &["app/models/user.rb"],
        );
    }

    #[test]
    fn keeps_explicit_non_rb_files() {
        let (_tmp, root) = setup(&[("Rakefile", "task :default")]);
        let rakefile = root.join("Rakefile");
        let files = collect_files(std::slice::from_ref(&rakefile), &root, &[]).unwrap();
        assert_eq!(files, vec![rakefile.clone()]);
    }

    #[test]
    fn keeps_explicit_files_with_other_extensions() {
        let (_tmp, root) = setup(&[
            ("lib/tasks/build.rake", "task :build"),
            ("gem.gemspec", "Gem::Specification.new"),
        ]);
        let rake = root.join("lib/tasks/build.rake");
        let gemspec = root.join("gem.gemspec");
        let files = collect_files(&[rake.clone(), gemspec.clone()], &root, &[]).unwrap();
        assert_eq!(files, vec![gemspec, rake]);
    }

    #[test]
    fn explicit_files_alongside_a_directory_walk() {
        let (_tmp, root) = setup(&[
            ("LICENSE", "MIT License"),
            ("Rakefile", "task :default"),
            ("lib/x.rb", "1"),
            ("lib/README", "docs"),
        ]);
        let files = collect_files(&[root.join("lib"), root.join("Rakefile")], &root, &[]).unwrap();
        assert_files(
            &files
                .iter()
                .map(|p| display_path(p, &root))
                .collect::<Vec<_>>(),
            &["Rakefile", "lib/x.rb"],
        );
    }

    #[test]
    fn excludes_still_apply_to_explicit_files() {
        let (_tmp, root) = setup(&[("vendor/tasks/x.rake", "task :x")]);
        let excludes = compile_globs(&["vendor/**".to_string()], "test").unwrap();
        let files = collect_files(&[root.join("vendor/tasks/x.rake")], &root, &excludes).unwrap();
        assert!(files.is_empty(), "{files:?}");
    }

    #[test]
    fn errors_on_missing_path() {
        let err =
            collect_files(&[PathBuf::from("/no/such/path/here")], Path::new(""), &[]).unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }
}
