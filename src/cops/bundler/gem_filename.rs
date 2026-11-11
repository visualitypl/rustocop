//! `Bundler/GemFilename`: fixed Gemfile style, lexical supplied names only.
use std::path::Path;

use crate::{cops::Cop, offense::GlobalOffense};

pub struct GemFilename;

impl Cop for GemFilename {
    fn name(&self) -> &'static str {
        "Bundler/GemFilename"
    }

    fn check_filename(&self, path: &Path, out: &mut Vec<GlobalOffense>) {
        let message = match path.file_name().and_then(|name| name.to_str()) {
            Some("gems.rb") => "`gems.rb` file was found but `Gemfile` is required",
            Some("gems.locked") => {
                "Expected a `Gemfile.lock` with `Gemfile` but found `gems.locked` file"
            }
            _ => return,
        };
        out.push(GlobalOffense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("{message} (file path: {}).", path.display()),
        });
    }
}

#[cfg(test)]
mod tests {
    //! RuboCop gem_filename_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;

    #[test]
    fn exact_basename_not_substrings_or_companion_existence() {
        for path in [
            "Gemfile",
            "Gemfile.lock",
            "gems.rb.bak",
            "Gems.rb",
            "gems.locked.rb",
            "other.gemfile",
            "gems.rb/other.rb",
            "gems.locked/other.lock",
        ] {
            let mut out = vec![];
            GemFilename.check_filename(Path::new(path), &mut out);
            assert!(out.is_empty(), "{path}");
        }
        for path in [
            "gems.rb",
            "missing/../nested/gems.rb",
            "/absent/gems.locked",
        ] {
            let mut out = vec![];
            GemFilename.check_filename(Path::new(path), &mut out);
            assert_eq!(out.len(), 1);
            assert!(out[0].message.ends_with(&format!("(file path: {path}).")));
        }
    }
}
