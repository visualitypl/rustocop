//! Prints every registered cop name, one per line, for the Ruby tools. With
//! `--settings`, prints the settings that the cops declare instead, one per
//! line: the cop, the setting, its kind and, for a choice, the values, with
//! tabs between them.
use rustocop::cops::{Registry, settings::Kind};

fn main() {
    let settings = std::env::args().any(|arg| arg == "--settings");
    for cop in Registry::new().cops() {
        if !settings {
            println!("{}", cop.name());
        }
        for setting in cop.settings().iter().filter(|_| settings) {
            let kind = match setting.kind {
                Kind::Count => "count".to_owned(),
                Kind::Choice(choices) => format!("choice\t{}", choices.join(",")),
                Kind::Patterns => "patterns".to_owned(),
                Kind::Flag => "flag".to_owned(),
            };
            println!("{}\t{}\t{kind}", cop.name(), setting.name);
        }
    }
}
