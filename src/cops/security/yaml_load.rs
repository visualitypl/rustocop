//! `Security/YAMLLoad` — registered, but never reports.
//!
//! RuboCop's cop flags `YAML.load` (suggesting `YAML.safe_load`) only when
//! `TargetRubyVersion` is 3.0 or older: since Ruby 3.1 (Psych 4)
//! `YAML.load` is `safe_load`, and `YAML.unsafe_load` is the explicit opt-in.
//! rustocop has no target-version setting and assumes modern Ruby
//!, as RuboCop's own Prism parser engine does (it requires
//! `TargetRubyVersion` 3.3 or newer). Flagging `YAML.load` would be a false
//! positive on every supported Ruby, so the cop is inert, as RuboCop's is for
//! these targets.
//!
//! It is registered so that configurations and `rubocop:disable` comments
//! naming it stay valid.
//! Its pinned maximum Ruby 3.0 bound also prevents dispatch at our fixed target;
//! no legacy-version matcher or public version setting is introduced.

use crate::cops::Cop;

/// The `Security/YAMLLoad` cop.
pub struct YamlLoad;

impl Cop for YamlLoad {
    fn name(&self) -> &'static str {
        "Security/YAMLLoad"
    }
}

#[cfg(test)]
mod tests {
    //! No-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/security/yaml_load_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov): the
    //! version-independent ones and the `Ruby >= 3.1` context. The
    //! `Ruby <= 3.0` context does not apply (see the module docs).

    use crate::{
        config::{RawConfig, ResolvedConfig},
        cops::{Registry, select_active},
        pipeline::{lint_str, lint_str_with},
    };

    fn found(source: &str) -> Vec<(u32, u32, String)> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Security/YAMLLoad")
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    #[test]
    fn accepts_rubocop_spec_no_offenses() {
        for source in [
            "YAML.dump(\"foo\")\n::YAML.dump(\"foo\")\nModule::YAML.dump(\"foo\")\n",
            "Module::YAML.load(\"foo\")\n",
            "YAML.load(\"--- !ruby/object:Foo {}\", permitted_classes: [Foo])\n",
            "::YAML.load(\"--- !ruby/object:Foo {}\", permitted_classes: [Foo])\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn yaml_load_is_safe_on_modern_ruby() {
        // RuboCop flags these only for TargetRubyVersion <= 3.0.
        for source in [
            "YAML.load(\"--- !ruby/object:Foo {}\")\n",
            "::YAML.load(\"--- foo\")\n",
            "YAML.load(File.read(path))\n",
            "YAML.unsafe_load(data)\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn inert_even_when_selected_alone() {
        let registry = Registry::new();
        let config = ResolvedConfig::default();
        let active = select_active(&registry, &config, &["Security/YAMLLoad".into()], &[]);
        assert!(
            active.is_empty(),
            "selection cannot bypass the Ruby 3.0 maximum"
        );
        assert!(lint_str_with(&active, "YAML.load(data)\n").is_empty());
    }

    #[test]
    fn configuration_may_name_it() {
        let registry = Registry::new();
        let raw: RawConfig =
            toml::from_str("[cops.\"Security/YAMLLoad\"]\nenabled = false\n").unwrap();
        assert!(ResolvedConfig::resolve(raw, None, &registry).is_ok());
    }
}
