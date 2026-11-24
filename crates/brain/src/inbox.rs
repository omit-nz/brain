//! The inbox: what was captured and not yet kept. Keeping moves a note out unchanged; skipping
//! archives it. Nothing here deletes.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use brain_note::gate::{self, GateFacts};
use brain_note::{Entry, Note, Place, Split, Value, Vault, rel_key};
use brain_platform::fs::{archive, rename_within};
use clap::ValueEnum;

use crate::out::terminal;

/// learning-loop's folders a kept note moves to.
#[derive(Clone, Copy, ValueEnum)]
pub enum Dest {
    Fleeting,
    Permanent,
    Literature,
    Maps,
}

impl Dest {
    fn folder(self) -> &'static str {
        match self {
            Dest::Fleeting => "1-fleeting",
            Dest::Literature => "2-literature",
            Dest::Permanent => "3-permanent",
            Dest::Maps => "5-maps",
        }
    }
}

struct Item {
    rel: PathBuf,
    id: Option<uuid::Uuid>,
    date: String,
    title: String,
    facts: GateFacts,
}

/// Inbox notes, oldest `date` first, then by path.
fn items(vault: &Vault) -> Vec<Item> {
    let mut out: Vec<Item> = vault
        .walk()
        .filter_map(|e| match e {
            Entry::Note {
                rel,
                place: Place::Inbox,
            } => Some(rel),
            _ => None,
        })
        .filter_map(|rel| {
            let raw = vault.read(&rel).ok()?;
            let split = Split::parse(&raw);
            let note = Note::parse(&rel, &raw, Place::Inbox);
            let date = match split.fm.as_ref().ok()?.as_ref().and_then(|f| f.get("date")) {
                Some(Value::Text(d)) => d,
                _ => String::new(),
            };
            Some(Item {
                facts: gate::facts(&note, &split),
                id: note.id,
                title: note.title,
                rel,
                date,
            })
        })
        .collect();
    out.sort_by(|a, b| (&a.date, &a.rel).cmp(&(&b.date, &b.rel)));
    out
}

/// The inbox note `target` names, by id or by its path in the vault.
fn find(vault: &Vault, target: &str) -> anyhow::Result<Item> {
    let id = uuid::Uuid::parse_str(target).ok();
    items(vault)
        .into_iter()
        .find(|i| match id {
            Some(id) => i.id == Some(id),
            None => rel_key(&i.rel) == target,
        })
        .with_context(|| format!("no note in the inbox is {target}"))
}

pub fn list(vault: &Vault) -> anyhow::Result<()> {
    let items = items(vault);
    if items.is_empty() {
        println!("the inbox is empty");
    }
    for i in &items {
        let f = &i.facts;
        let date = if i.date.is_empty() {
            "no date"
        } else {
            &i.date
        };
        let line = format!(
            "{date}  {}  {}\n    markers: {}; links {}; source {}; {} lines; missing: {}; route: {:?}",
            rel_key(&i.rel),
            i.title,
            if f.markers.is_empty() {
                "none".into()
            } else {
                f.markers.join(" ")
            },
            f.wikilinks,
            if f.has_source { "yes" } else { "no" },
            f.body_lines,
            if f.missing_keys.is_empty() {
                "none".into()
            } else {
                f.missing_keys.join(", ")
            },
            gate::route(f, None, false),
        );
        println!("{}", terminal(&line));
    }
    Ok(())
}

pub fn keep(vault: &Vault, target: &str, to: Dest) -> anyhow::Result<()> {
    let item = find(vault, target)?;
    let Some(name) = item.rel.file_name() else {
        bail!("{} has no file name", rel_key(&item.rel));
    };
    let dest = Path::new(to.folder()).join(name);
    rename_within(&vault.root, &item.rel, &dest)
        .with_context(|| format!("moving {} to {}", rel_key(&item.rel), rel_key(&dest)))?;
    println!("{}", terminal(&format!("kept: {}", rel_key(&dest))));
    Ok(())
}

pub fn skip(vault: &Vault, target: &str) -> anyhow::Result<()> {
    let item = find(vault, target)?;
    let moved = archive(&vault.root, &item.rel, &vault.layout.archive)?;
    println!("{}", terminal(&format!("archived: {}", rel_key(&moved))));
    Ok(())
}
