use super::*;

struct PositionEdit(crate::offense::Correction);
impl PositionEdit {
    fn emit(&self, out: &mut Vec<Offense>, message: &str) {
        out.push(Offense {
            cop: "Style/TestPositionEmission",
            severity: crate::offense::Severity::Convention,
            line: 1,
            col: 1,
            end_line: 1,
            message: message.into(),
            correction: Some(self.0.clone()),
        });
    }
}
impl super::Cop for PositionEdit {
    fn name(&self) -> &'static str {
        "Style/TestPositionEmission"
    }
    fn check_source(&self, _: &SourceFile, out: &mut Vec<Offense>) {
        self.emit(out, "source");
    }
    fn wants_leave_node(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &ruby_prism::Node<'pr>,
        _: &super::NodeContext<'_, 'pr>,
        _: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_program_node().is_some() {
            self.emit(out, "node");
        }
    }
    fn leave_node<'pr>(
        &self,
        node: &ruby_prism::Node<'pr>,
        _: &super::NodeContext<'_, 'pr>,
        _: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_program_node().is_some() {
            self.emit(out, "leave");
        }
    }
}
fn position_edit(source: &str, range: std::ops::Range<usize>, replacement: &str, safe: bool) {
    let found = collect(
        &PositionEdit(crate::offense::Correction {
            range,
            replacement: replacement.into(),
        }),
        source,
    );
    assert_eq!(found.len(), 3);
    assert_eq!(
        found.iter().map(|o| o.message.as_str()).collect::<Vec<_>>(),
        ["source", "node", "leave"]
    );
    assert!(
        found.iter().all(|o| o.correction.is_some() == safe),
        "{source}: {replacement:?}"
    );
}

#[test]
fn source_node_leave_positions_filter_each_delta_not_whole_files() {
    position_edit("1\np File.size('case.rb')\n", 0..1, "22", false);
    position_edit("1\np File.stat('./case.rb').mtime\n", 0..1, "2", false);
    for observer in [
        "p test(?s, 'case.rb')",
        "p File.send(:size, 'case.rb')",
        "p File.public_send(:send, 'size', 'case.rb')",
    ] {
        let source = format!("1\n{observer}\n");
        position_edit(&source, 0..1, "22", false);
        position_edit(&source, 0..1, "2", true);
    }
    for observer in [
        "p Kernel.test(?M, 'case.rb')",
        "p File.send(:stat, 'case.rb').mtime",
    ] {
        position_edit(&format!("1\n{observer}\n"), 0..1, "2", false);
    }
    // A listed observer (`__FILE__`,
    // `DATA` with a data section, `caller_locations`, `source_location`)
    // withholds every edit, even one the position barrier would admit.
    position_edit(
        "1\np [__FILE__, File.size('unrelated')]\n",
        0..1,
        "22",
        false,
    );
    for observer in [
        "p DATA.pos\n__END__\npayload",
        "p DATA.tell\n__END__\npayload",
        "p File.size(__FILE__)",
        "p DATA.lineno\n__END__\npayload",
        "p caller_locations(0)",
        "p method(:f).source_location",
    ] {
        let source = format!("1\n{observer}\n");
        position_edit(&source, 0..1, "22", false);
        position_edit(&source, 0..1, "2", false);
        position_edit(&source, 1..2, "\n\n", false);
    }
    for observer in [
        "p __LINE__",
        "TracePoint.new {}",
        "Object.const_get(:TracePoint).new {}",
        "Object.const_get('::TracePoint').new {}",
    ] {
        let source = format!("1\n{observer}\n");
        position_edit(&source, 1..2, "\n\n", false);
        position_edit(&source, 0..1, "22", true);
    }
    // Reflected const_get also meets the pre-existing raw-source barrier;
    // do not claim its byte-only sibling is independently proven safe.
    position_edit(
        "1\nObject.send(:const_get, :TracePoint).new {}\n",
        1..2,
        "\n\n",
        false,
    );
    for observer in [
        "p File.mtime(__FILE__)",
        "p DATA.stat.mtime\n__END__\npayload",
    ] {
        position_edit(&format!("1\n{observer}\n"), 0..1, "2", false);
    }
}

#[test]
fn filesystem_uncertainty_is_shared_by_source_node_and_leave_emissions() {
    for observer in [
        "p File.size(File.join(__dir__, 'case.rb'))",
        "File.symlink(__FILE__, 'alias'); p File.size('alias')",
    ] {
        // Listed `__dir__` / `__FILE__`: every edit is held.
        let source = format!("1\n{observer}\n");
        position_edit(&source, 0..1, "22", false);
        position_edit(&source, 0..1, "2", false);
    }
    for observer in [
        "p File.size(File.join(Dir.pwd, 'case.rb'))",
        "p File.size('case' + '.rb')",
        "p test(?s, 'case' + '.rb')",
        "F = File; p F.size(path)",
        "p File.stat(path).size",
        "p File.send(:stat, path).public_send(:size)",
        "File.symlink(path, 'alias'); p File.size('alias')",
    ] {
        let source = format!("1\n{observer}\n");
        position_edit(&source, 0..1, "22", false);
        position_edit(&source, 0..1, "2", true);
    }
    for observer in [
        "p File.stat(__FILE__)",
        "p File.stat(path)",
        "p File.lstat('case.rb')",
        "p DATA.stat\n__END__\npayload",
        "saved = File.stat(__FILE__); escape(saved)",
        "p File.stat(__FILE__).method(:size).receiver",
        "getter = File.method(:stat); escape(getter)",
        "p File::Stat.new(path)",
        "S = File::Stat; escape(S.new(path))",
        "p File.atime(path)",
        "p test(?A, path)",
        "p test(65, path)",
        "getter = method(:test); escape(getter)",
        "alias metadata test; escape(method(:metadata))",
        "File.link(__FILE__, 'alias'); p File.stat('alias')",
        "p File.identical?('other', 'case.rb')",
        "p test(?=, 'other', 'case.rb')",
        "p test(60, path, 'other')",
        "p test(62, 'other', path)",
    ] {
        let source = format!("1\n{observer}\n");
        position_edit(&source, 0..1, "22", false);
        position_edit(&source, 0..1, "2", false);
    }
    for observer in [
        "p DATA.stat.file?\n__END__\npayload",
        "p File.stat(__FILE__).ftype",
        "p File.lstat(__FILE__).symlink?",
        "p [__FILE__, Dir.children('.').size, [1].size, {a: 1}.size]",
    ] {
        // Listed observers: every edit is held.
        position_edit(&format!("1\n{observer}\n"), 0..1, "22", false);
    }
    for observer in [
        "p File.stat(path).ftype",
        "p File.lstat(path).symlink?",
        "p File.stat('unrelated')",
        "p File.atime('unrelated')",
        "p File.identical?('other', 'unrelated')",
        "p test(?=, 'other', 'unrelated')",
        "p [Dir.children('.').size, [1].size, {a: 1}.size]",
    ] {
        position_edit(&format!("1\n{observer}\n"), 0..1, "22", true);
    }
}

#[test]
fn explicit_and_unresolved_test_receivers_classify_paths_and_flags() {
    for receiver in ["self", "receiver"] {
        for dispatch in ["test", "send(:test,", "public_send(:send, :test,"] {
            let call = |flag, path| {
                if dispatch == "test" {
                    format!("{receiver}.test({flag}, {path})")
                } else {
                    format!("{receiver}.{dispatch} {flag}, {path})")
                }
            };
            for path in ["'case.rb'", "'case' + '.rb'", "path"] {
                let source = format!("1\np {}\n", call("?s", path));
                position_edit(&source, 0..1, "22", false);
                position_edit(&source, 0..1, "2", true);
                let source = format!("1\np {}\n", call("?M", path));
                position_edit(&source, 0..1, "2", false);
            }
            for flag in ["?s", "?M"] {
                let source = format!("1\np {}\n", call(flag, "'unrelated'"));
                position_edit(&source, 0..1, "22", true);
            }
        }
        position_edit(&format!("1\n{receiver}.test(*args)\n"), 0..1, "2", false);
        for args in ["", "?s"] {
            position_edit(&format!("1\n{receiver}.test({args})\n"), 0..1, "22", true);
        }
    }
}

#[test]
fn relative_symlink_acquisition_invalidates_unrelated_paths() {
    for acquisition in [
        "FileUtils.ln_sr(__FILE__, 'alias')",
        "FileUtils.send(:ln_sr, __FILE__, 'alias')",
        "FileUtils.public_send(:send, 'ln_sr', __FILE__, 'alias')",
        "getter = FileUtils.method(:ln_sr)",
        "getter = :ln_sr.to_proc",
        "class << FileUtils; alias make_alias ln_sr; end",
    ] {
        // A listed `__FILE__` holds every edit.
        let listed = acquisition.contains("__FILE__");
        let source = format!("1\n{acquisition}; p File.size('alias')\n");
        position_edit(&source, 0..1, "22", false);
        position_edit(&source, 0..1, "2", !listed);
        let source = format!("1\n{acquisition}; p File.mtime('alias')\n");
        position_edit(&source, 0..1, "2", false);
    }
    for inert in ["p :ln_sr", "p 'ln_sr'", "# FileUtils.ln_sr"] {
        let source = format!("1\n{inert}\np File.size('unrelated')\n");
        position_edit(&source, 0..1, "22", true);
    }
}

#[test]
fn move_alias_acquisition_filters_source_node_and_leave_by_metadata_delta() {
    for name in ["mv", "move"] {
        for acquisition in [
            format!("FileUtils.{name}(__FILE__, 'alias')"),
            format!("FileUtils.send(:{name}, __FILE__, 'alias')"),
            format!("FileUtils.public_send(:send, '{name}', __FILE__, 'alias')"),
            format!("getter = FileUtils.method(:{name})"),
            format!("getter = :{name}.to_proc"),
            format!("class << FileUtils; alias relocate {name}; end"),
            format!("FileUtils.singleton_class.alias_method(:relocate, :{name})"),
            format!("include FileUtils; {name}(__FILE__, 'alias')"),
            format!("def later; FileUtils.{name}(__FILE__, 'alias'); end"),
        ] {
            // A listed `__FILE__` holds every edit.
            let listed = acquisition.contains("__FILE__");
            let source = format!("1\n{acquisition}; p File.size('alias')\n");
            position_edit(&source, 0..1, "22", false);
            position_edit(&source, 0..1, "2", !listed);
            let source = format!("1\n{acquisition}; p File.mtime('alias')\n");
            position_edit(&source, 0..1, "2", false);
        }
        let projection = format!("FileUtils.{name}(__FILE__, 'alias'); p File.stat('alias').size");
        position_edit(&format!("1\n{projection}\n"), 0..1, "2", false);
        let projection = format!("FileUtils.{name}(path, 'alias'); p File.stat('alias').size");
        position_edit(&format!("1\n{projection}\n"), 0..1, "2", true);
        // A visible load revokes the existing independent heap metadata
        // exemption; equal bytes are only the positional half of the proof.
        position_edit(
            &format!("1\nrequire 'fileutils'; {projection}\n"),
            0..1,
            "2",
            false,
        );
        for inert in [
            format!("p :{name}"),
            format!("p '{name}'"),
            format!("# FileUtils.{name}"),
        ] {
            let source = format!("1\n{inert}\np File.size('unrelated')\n");
            position_edit(&source, 0..1, "22", true);
        }
    }
}

#[test]
fn parsed_line_markers_inside_atomic_spans_keep_their_own_ordinals() {
    let source = "p [\n__LINE__]\n";
    position_edit(source, 0..source.len(), "p [__LINE__\n]\n", false);
    position_edit(source, 0..source.len(), "p([\n__LINE__])\n", true);
    // Equal totals can still move method definitions / trace events internally.
    let source = "def f;\n1; end\np method(:f).source_location\n";
    position_edit(source, 0..14, "\ndef f; 1; end\n", false);
}

#[test]
fn inert_metadata_spellings_and_unrelated_handles_do_not_block_edits() {
    // Listed observers hold every edit; the
    // symbol `:source_location` is a conservative listed selector.
    for observer in [
        "p [:pos, :tell, :mtime, :source_location]",
        "p [__FILE__, test(?s, '/dev/null'), File.send(:size, '/dev/null')]",
        "p DATA.read\n__END__\npayload DATA.pos __LINE__",
    ] {
        position_edit(&format!("1\n{observer}\n"), 0..1, "22", false);
        position_edit(&format!("1\n{observer}\n"), 1..2, "\n\n", false);
    }
    for observer in [
        "p [:pos, :tell, :mtime]",
        "p 'DATA.pos __LINE__ File.mtime(__FILE__)'",
        "# DATA.tell __LINE__ caller_locations",
        "p File.size('/dev/null')",
        "p [test(?s, '/dev/null'), File.send(:size, '/dev/null')]",
        "p Object.const_get(:String)",
        "p :TracePoint",
        "def pos; 1; end; p pos",
    ] {
        position_edit(&format!("1\n{observer}\n"), 0..1, "22", true);
        position_edit(&format!("1\n{observer}\n"), 1..2, "\n\n", true);
    }
}

#[test]
fn data_positions_depend_on_prefix_not_unobserved_payload_edits() {
    // `DATA` with a data section is listed: every edit is held.
    let source = "p DATA.pos\n__END__\npayload\n";
    let start = source.find("payload").unwrap();
    position_edit(source, start..start + 7, "longer payload", false);
    let source = "p __LINE__\n1\n";
    position_edit(source, source.len()..source.len(), "\n", true);
}
