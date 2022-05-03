#![feature(once_cell)]

mod symbol_collect_visitor;

use anyhow::Result;
use itertools::Itertools;
use proc_macro2::Ident;
use ra_ap_base_db::{Change, FileId, FilePosition, VfsPath};
use ra_ap_ide::{AnalysisHost, LineCol, LineIndex, TextEdit};
use ra_ap_project_model::CargoConfig;
use ra_ap_rust_analyzer::cli::load_cargo::{load_workspace_at, LoadCargoConfig};
use std::{collections::HashSet, lazy::SyncLazy, sync::Arc};
use symbol_collect_visitor::SymbolCollectVisitor;
use syn::visit_mut::VisitMut;

static FIRST_CHARS: SyncLazy<Vec<char>> = SyncLazy::new(|| {
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
        .chars()
        .collect_vec()
});

static CHARS: SyncLazy<Vec<char>> = SyncLazy::new(|| {
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_0123456789"
        .chars()
        .collect_vec()
});

pub struct SymbolNameGenerator {
    used_symbol: HashSet<String>,
    current: Vec<usize>,
    reserved_words: HashSet<String>,
}

impl SymbolNameGenerator {
    pub fn new(used_idents: &Vec<Ident>, other_idents: &Vec<Ident>) -> Self {
        Self {
            used_symbol: HashSet::from_iter(
                used_idents
                    .iter()
                    .chain(other_idents)
                    .map(|i| i.to_string()),
            ),
            current: Vec::new(),
            reserved_words: HashSet::from_iter(
                vec![
                    "as", "impl", "fn", "do", "while", "if", "else", "match", "use", "type",
                    "enum", "struct", "pub", "crate", "super", "self", "Self", "static", "mod",
                    "let", "const", "mut",
                ]
                .into_iter()
                .map(str::to_owned),
            ),
        }
    }

    pub fn gen_next_symbol(&mut self) -> String {
        if self.current.is_empty() {
            self.current.push(0);
            return FIRST_CHARS[0].to_string();
        }
        for i in 0..self.current.len() {
            self.current[i] += 1;

            let is_first_digit = i == self.current.len() - 1;
            let valid_symbol_char_count = if is_first_digit {
                FIRST_CHARS.len()
            } else {
                CHARS.len()
            };

            if self.current[i] != valid_symbol_char_count {
                break;
            }

            self.current[i] = 0;
            if is_first_digit {
                self.current.push(0);
                break;
            }
        }
        self.current.iter().rev().map(|&c| CHARS[c]).collect()
    }

    pub fn gen_next_unused_symbol(&mut self) -> String {
        loop {
            let symbol = self.gen_next_symbol();
            if !self.used_symbol.contains(&symbol) && !self.reserved_words.contains(&symbol) {
                break symbol;
            }
        }
    }
}

// 余分な空白を削除する
fn remove_extra_space(source: String) -> String {
    let cs = source.chars().collect_vec();
    let mut result = Vec::new();

    let mut i = 0;
    loop {
        while i < cs.len() && cs[i].is_whitespace() {
            i += 1;
        }

        if i >= cs.len() {
            break result.into_iter().collect();
        }

        if cs[i..].starts_with(&['r', '#', '"']) {
            while !result.ends_with(&['"', '#']) {
                result.push(cs[i]);
                i += 1;
            }
            continue;
        }

        if cs[i..].starts_with(&['/', '/']) {
            while !result.ends_with(&['\n']) {
                result.push(cs[i]);
                i += 1;
            }
            continue;
        }

        fn alnum(c: char) -> bool {
            c.is_alphanumeric() || c == '_'
        }

        if result.ends_with(&['.', '0']) && cs[i] == '.' {
            result.push(' ');
        } else if let Some(last) = result.last() {
            if alnum(*last) && alnum(cs[i]) {
                result.push(' ');
            }
        }

        result.push(cs[i]);
        i += 1;
        while alnum(cs[i]) {
            result.push(cs[i]);
            i += 1;
        }
    }
}

fn minify_file_with_idents(
    analysis_host: &mut AnalysisHost,
    file_id: FileId,
    rename_idents: Vec<Ident>,
    other_idents: Vec<Ident>,
) -> Result<String> {
    let file_text = {
        let analysis = analysis_host.analysis();

        let mut file_text = (*analysis.file_text(file_id).unwrap()).clone();
        let line_index = LineIndex::new(&file_text);

        let mut symbol_name_generator = SymbolNameGenerator::new(&rename_idents, &other_idents);
        let mut next_symbol = symbol_name_generator.gen_next_unused_symbol();

        let mut builder = TextEdit::builder();

        for rename_ident in rename_idents {
            let renamed_symbol = rename_ident.to_string();

            // main関数の名前は変えられないのでスキップ
            if renamed_symbol == "main" {
                continue;
            }

            if renamed_symbol.to_string().len() <= next_symbol.len() {
                continue;
            }

            let start = rename_ident.span().start();
            let line_col = LineCol {
                line: start.line as u32 - 1, // 1-indexed から 0-indexed への変換
                col: start.column as u32,
            };
            let offset = line_index.offset(line_col).unwrap();
            let position = FilePosition { file_id, offset };

            let source_change = analysis.rename(position, &next_symbol)?;
            for text_edit in source_change.unwrap().source_file_edits.values() {
                for indel in text_edit.iter() {
                    builder.replace(indel.delete, indel.insert.clone());
                }
            }

            next_symbol = symbol_name_generator.gen_next_unused_symbol();
        }

        builder.finish().apply(&mut file_text);

        file_text
    };

    let mut change = Change::default();
    change.change_file(file_id, Some(Arc::new(file_text.clone())));
    analysis_host.apply_change(change);

    Ok(file_text)
}

fn minify_file(analysis_host: &mut AnalysisHost, file_id: FileId) -> Result<String> {
    fn create_symbol_collect_visitor(
        analysis_host: &AnalysisHost,
        file_id: FileId,
    ) -> SymbolCollectVisitor {
        log::info!("Parse file start.");

        let analysis = analysis_host.analysis();

        let file_text = (*analysis.file_text(file_id).unwrap()).clone();
        let mut visitor = SymbolCollectVisitor::new();
        let mut syntax = syn::parse_file(&file_text).unwrap();
        visitor.visit_file_mut(&mut syntax);

        log::info!("Parse file done.");

        visitor
    }

    // struct X { field: usize }
    // let field = 0;
    // let x = X { field };
    //
    // というコードで、ローカル変数のfieldとXのフィールド名のfieldをそれぞれa, bにリネームする際に
    //
    // 1.
    //   let x = X { field: a };
    //
    // 2.
    //   let x = X { b: a };
    //
    // と2段階にわけてリネームする必要があるので、以下のように2回minify_file_with_identsを呼び出している

    let visitor = create_symbol_collect_visitor(analysis_host, file_id);

    log::info!("Rename variable idents.");
    minify_file_with_idents(
        analysis_host,
        file_id,
        visitor.ident_var,
        visitor.ident_others,
    )?;

    let visitor = create_symbol_collect_visitor(analysis_host, file_id);

    log::info!("Rename other idents.");
    let file_text = minify_file_with_idents(
        analysis_host,
        file_id,
        visitor.ident_others,
        visitor.ident_var,
    )?;

    Ok(remove_extra_space(file_text))
}

#[argopt::subcmd]
fn minify(#[opt(short, long)] root: String) -> Result<()> {
    let root = std::fs::canonicalize(root)?;

    fn ignore_progress(_: String) {}

    let (mut analysis_host, vfs, _proc_macro_server) = load_workspace_at(
        &root,
        &CargoConfig::default(),
        &LoadCargoConfig {
            load_out_dirs_from_check: true,
            with_proc_macro: false,
            prefill_caches: true,
        },
        &ignore_progress,
    )
    .unwrap();

    let root_vfs_path = VfsPath::new_real_path(root.to_str().unwrap().to_string());

    for (file_id, vfs_path) in vfs.iter() {
        if !vfs_path.starts_with(&root_vfs_path) {
            continue;
        }

        // 今の所複数ファイルには未対応
        if vfs_path.as_path().unwrap().file_name().unwrap() != "main.rs" {
            continue;
        }

        let file_text = minify_file(&mut analysis_host, file_id)?;

        println!("{}", file_text);
    }

    Ok(())
}

#[argopt::cmd_group(commands = [minify])]
fn main() -> Result<()> {
    env_logger::init();
}

#[cfg(test)]
mod tests {}
