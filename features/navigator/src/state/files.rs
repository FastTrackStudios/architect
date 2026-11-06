//! File operations (Notebook Navigator's `FileSystemService`,
//! `fileCreationUtils.ts`, `noteMerge.ts`, the open-all command): notes,
//! folders, canvases and bases made, from a template or not; renamed,
//! duplicated, moved, copied, put in the trash and merged. Each returns an
//! [`Outcome::Write`] for the source to do, as the tag operations do.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::{DateTime, NaiveDate};

use super::{Drag, NavigatorState, OpenIn, Outcome, join};
use crate::period::PeriodKind;
use crate::selection::NavTarget;
use crate::source::{FileOp, is_within, parent};
use crate::template::{
    self, CommandLocation, Context, Number, WeekdayBase, folder_template, format_numbered,
    in_folder, next_number, numbered_name, unique_name,
};

/// Opening this many notes at once asks first (`OPEN_ALL_FILES_WARNING_THRESHOLD`).
pub const OPEN_ALL_WARNING: usize = 15;

/// Reads a file's text for the navigator (a template, notes merged): the
/// source's [`crate::NavigatorSource::read_text`].
#[derive(Clone)]
pub struct Reader(Option<Arc<dyn Fn(&str) -> Option<String> + Send + Sync>>);

impl Reader {
    /// Reads nothing: templates make empty notes.
    #[must_use]
    pub const fn none() -> Self {
        Self(None)
    }

    pub fn new(read: impl Fn(&str) -> Option<String> + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(read)))
    }

    #[must_use]
    pub fn read(&self, path: &str) -> Option<String> {
        self.0.as_ref().and_then(|r| r(path))
    }
}

impl std::fmt::Debug for Reader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "Reader"
        } else {
            "Reader(none)"
        })
    }
}

/// What a name typed for a note or a folder is (`fileNameUtils.ts`): an
/// error stops it; a warning is said and it goes ahead.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NameCheck {
    pub error: Option<String>,
    pub warning: Option<String>,
}

impl NameCheck {
    #[must_use]
    pub const fn ok(&self) -> bool {
        self.error.is_none()
    }
}

/// A name checked: empty, a leading period, `:` or `/` (and on Windows
/// `< > " \ | ? *`) cannot be; `#`, `|`, `^`, `%%`, `[[`, `]]` break
/// links, so they are warned about.
#[must_use]
pub fn check_name(name: &str) -> NameCheck {
    let name = name.trim();
    let mut c = NameCheck::default();
    if name.is_empty() {
        c.error = Some("A name is needed".to_owned());
    } else if name.starts_with('.') || name.contains([':', '/']) {
        c.error = Some("Names cannot start with a period or include : or /".to_owned());
    } else if cfg!(windows) && name.contains(['<', '>', '"', '\\', '|', '?', '*']) {
        c.error =
            Some("Windows-reserved characters are not allowed: <, >, \", \\, |, ?, *".to_owned());
    }
    if name.contains(['#', '|', '^'])
        || name.contains("%%")
        || name.contains("[[")
        || name.contains("]]")
    {
        c.warning =
            Some("This name includes characters that break links: #, |, ^, %%, [[, ]]".to_owned());
    }
    c
}

/// What a new file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NewKind {
    #[default]
    Note,
    /// An Obsidian canvas (`{}`).
    Canvas,
    /// An Obsidian base (one table view).
    Base,
}

impl NewKind {
    const fn extension(self) -> &'static str {
        match self {
            Self::Note => "md",
            Self::Canvas => "canvas",
            Self::Base => "base",
        }
    }

    const fn text(self) -> &'static str {
        match self {
            Self::Note => "",
            Self::Canvas => "{}",
            Self::Base => "views:\n  - type: table\n    name: Table\n",
        }
    }
}

/// A new note asked for: in `folder`, from `template` (a vault path; none,
/// the folder's template), named by `name_format` (tokens; none,
/// `Untitled`), with the answers to its prompts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewNote {
    pub folder: String,
    pub kind: NewKind,
    pub template: Option<String>,
    /// No folder template either.
    pub no_template: bool,
    pub name_format: Option<String>,
    /// A name typed; it beats the format.
    pub name: Option<String>,
    pub prompts: BTreeMap<String, String>,
    pub how: OpenIn,
    /// The day `{{date}}` is (a periodic note's first); today when none.
    pub date: Option<NaiveDate>,
}

/// What asking for a note gives: the prompts to answer first, or the
/// write and where the editor's cursor goes (`{{cursor}}`'s line and
/// column).
#[derive(Debug, Clone, PartialEq)]
pub enum Made {
    Ask(Vec<String>),
    Done {
        out: Outcome,
        cursor: Option<(usize, usize)>,
        /// What to tell the person: the template did not read, or has
        /// tokens it did not write.
        warning: Option<String>,
    },
}

/// What goes between merged notes (`MergeNotesSeparator`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MergeSeparator {
    None,
    #[default]
    BlankLine,
    HorizontalRule,
    /// Each note after the first under a heading of its name.
    Heading,
}

impl MergeSeparator {
    /// `none`, `blank-line`, `horizontal-rule`, `heading`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(
            match s.trim().to_ascii_lowercase().replace('_', "-").as_str() {
                "none" => Self::None,
                "blank-line" | "blank" => Self::BlankLine,
                "horizontal-rule" | "rule" | "hr" => Self::HorizontalRule,
                "heading" => Self::Heading,
                _ => return None,
            },
        )
    }

    fn text(self, name: &str) -> String {
        match self {
            Self::None => "\n".to_owned(),
            Self::BlankLine => "\n\n".to_owned(),
            Self::HorizontalRule => "\n\n---\n\n".to_owned(),
            Self::Heading => format!("\n\n# {name}\n\n"),
        }
    }
}

/// `text` without the frontmatter it starts with (`stripLeadingFrontmatter`).
fn strip_frontmatter(text: &str) -> &str {
    let mut lines = text.split_inclusive('\n');
    if lines.next().map(str::trim) != Some("---") {
        return text;
    }
    let mut at = text.find('\n').map_or(text.len(), |n| n.saturating_add(1));
    for line in lines {
        at = at.saturating_add(line.len());
        if matches!(line.trim(), "---" | "...") {
            return text.get(at..).unwrap_or("");
        }
    }
    text
}

/// The notes' texts as one (`buildMergedNoteContent`): the first's
/// frontmatter kept, the others' left out, `sep` between.
#[must_use]
pub fn merged_text(notes: &[(String, String)], sep: MergeSeparator) -> String {
    let mut out = String::new();
    for (i, (name, text)) in notes.iter().enumerate() {
        if i == 0 {
            out.push_str(text.trim_end_matches(['\n', '\r']));
        } else {
            out.push_str(&sep.text(name));
            out.push_str(strip_frontmatter(text).trim_matches(['\n', '\r']));
        }
    }
    format!("{}\n", out.trim_end_matches(['\n', '\r']))
}

impl NavigatorState {
    /// Reads files' texts through `reader` (the source's): templates and
    /// notes to merge.
    pub fn set_reader(&mut self, reader: Reader) {
        self.reader = reader;
    }

    /// The folders' templates and the "create note" commands.
    #[must_use]
    pub const fn templates(&self) -> &crate::template::Templates {
        &self.templates
    }

    /// Changes the templates (a folder's set, a command added).
    pub fn edit_templates(&mut self, f: impl FnOnce(&mut crate::template::Templates)) {
        f(&mut self.templates);
    }

    /// Whether anything (shown or hidden) is at `path`.
    fn taken(&self, path: &str) -> bool {
        self.all.file(path).is_some() || self.all.has_folder(path)
    }

    /// Whether anything is at `path`, any case (two names that differ in
    /// case alone are one file on a Mac).
    fn taken_any_case(&self, path: &str) -> bool {
        let lower = path.to_lowercase();
        self.taken(path)
            || self.all.files().any(|f| f.path.to_lowercase() == lower)
            || self.all.folder_paths().any(|f| f.to_lowercase() == lower)
    }

    /// The folder a note is made in when none is named: the folder
    /// selected, else the open note's, else the root
    /// (`resolveTemplateCommandFolder`'s `current`).
    #[must_use]
    pub fn current_folder(&self) -> String {
        if let NavTarget::Folder(f) = self.target() {
            return f;
        }
        self.active
            .as_deref()
            .map_or_else(String::new, |a| parent(a).to_owned())
    }

    /// A template's text as the engine reads it: Templater's commands
    /// (`<% tp.… %>`) as tokens, unless the settings say the built-in
    /// engine alone (`templateEngine`).
    fn engine_text(&self, text: String) -> String {
        if self.settings.template_engine == crate::settings::TemplateEngine::BuiltIn {
            text
        } else {
            template::templater(&text)
        }
    }

    /// A template's text written for a note at `path`.
    fn render_for(
        &self,
        text: &str,
        path: &str,
        number: Number,
        date: Option<NaiveDate>,
        weekday_base: WeekdayBase,
        date_format: Option<&str>,
        prompts: &BTreeMap<String, String>,
    ) -> template::Rendered {
        let title = path
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .rsplit_once('.')
            .map_or(path, |(stem, _)| stem);
        let folder = parent(path);
        let folder_name = folder.rsplit('/').next().unwrap_or(folder);
        template::render(
            text,
            &self.template_context(
                title,
                folder_name,
                path,
                number,
                date,
                weekday_base,
                date_format,
                prompts,
            ),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn template_context<'a>(
        &'a self,
        title: &'a str,
        folder: &'a str,
        path: &'a str,
        number: Number,
        date: Option<NaiveDate>,
        weekday_base: WeekdayBase,
        date_format: Option<&'a str>,
        prompts: &'a BTreeMap<String, String>,
    ) -> Context<'a> {
        let local = self.clock.now.saturating_add(self.clock.offset);
        let now = DateTime::from_timestamp_millis(local)
            .unwrap_or_default()
            .naive_utc();
        Context {
            title,
            folder,
            path,
            date,
            date_format: date_format.unwrap_or(&self.settings.date_format),
            today_format: &self.settings.date_format,
            time_format: &self.settings.time_format,
            weekday_base,
            now,
            offset: i32::try_from(self.clock.offset / 1000).unwrap_or(0),
            weeks: self.periodic.weeks(),
            prompts,
            number,
        }
    }

    /// The template a note made in `folder` takes: `asked`, else the
    /// folder's (or the nearest above's that includes subfolders).
    #[must_use]
    pub fn template_for(&self, folder: &str, asked: Option<&str>) -> Option<String> {
        asked
            .map(str::to_owned)
            .or_else(|| folder_template(&self.templates.folders, folder).map(str::to_owned))
            .filter(|t| !t.trim().is_empty())
    }

    /// A new note, canvas or base (`createNewFile`, `createCanvas`,
    /// `createBase`, `createMarkdownFileFromTemplate`): named `Untitled`
    /// (then `Untitled 1`, …), by the name typed, or by a name format with
    /// tokens and numbers; a note's text from its template. Prompts in the
    /// name or the template are asked for first ([`Made::Ask`]).
    ///
    /// # Errors
    ///
    /// No such folder, a name that cannot be, or it is taken.
    pub fn new_note(&self, req: &NewNote) -> Result<Made, String> {
        let folder = req.folder.trim_matches('/');
        if !folder.is_empty() && !self.all.has_folder(folder) {
            return Err(format!("no folder {folder}"));
        }
        let ext = req.kind.extension();
        let template_path = (req.kind == NewKind::Note && !req.no_template)
            .then(|| self.template_for(folder, req.template.as_deref()))
            .flatten();
        // A template that does not read makes an empty note, and says so
        // (Notebook Navigator's `readFailed`).
        let text = template_path
            .as_deref()
            .and_then(|p| self.reader.read(p))
            .map(|t| self.engine_text(t));
        let mut warning = template_path
            .as_deref()
            .filter(|_| text.is_none())
            .map(|p| format!("The template {p} could not be read"));
        let mut asks = req
            .name_format
            .as_deref()
            .filter(|_| req.name.is_none())
            .map(template::prompts)
            .unwrap_or_default();
        for l in text.as_deref().map(template::prompts).unwrap_or_default() {
            if !asks.contains(&l) {
                asks.push(l);
            }
        }
        let missing: Vec<String> = asks
            .into_iter()
            .filter(|l| !req.prompts.contains_key(l))
            .collect();
        if !missing.is_empty() {
            return Ok(Made::Ask(missing));
        }
        // The name: typed, from the format (numbered), or Untitled.
        let (base, number) = if let Some(name) = &req.name {
            let check = check_name(name);
            if let Some(e) = check.error {
                return Err(e);
            }
            let name = name.trim();
            let name = name
                .strip_suffix(&format!(".{ext}"))
                .unwrap_or(name)
                .trim_end_matches('.')
                .to_owned();
            if self.taken_any_case(&in_folder(folder, &name, ext)) {
                return Err(format!(
                    "{} is there already",
                    in_folder(folder, &name, ext)
                ));
            }
            (name, None)
        } else if let Some(format) = req.name_format.as_deref().filter(|f| !f.trim().is_empty()) {
            let folder_name = folder.rsplit('/').next().unwrap_or(folder);
            let cx = self.template_context(
                "",
                folder_name,
                "",
                Number::Slot,
                req.date,
                WeekdayBase::LocaleWeek,
                None,
                &req.prompts,
            );
            let r = template::render(format, &cx);
            if !r.invalid.is_empty() {
                return Err(format!(
                    "The name format has tokens that do not read: {}",
                    r.invalid.join(" ")
                ));
            }
            let mut parts = numbered_name(&r.content, &r.number_slots);
            if parts.is_empty() {
                parts = vec![template::NamePart::Text("Untitled".to_owned())];
            }
            let n = next_number(folder, &parts, self.all.files().map(|f| f.path.as_str()));
            let name = format_numbered(&parts, n);
            (
                unique_name(folder, &name, ext, |p| self.taken_any_case(p)),
                n,
            )
        } else {
            (
                unique_name(folder, "Untitled", ext, |p| self.taken_any_case(p)),
                None,
            )
        };
        let path = in_folder(folder, &base, ext);
        let (content, cursor) = text.as_ref().map_or_else(
            || (req.kind.text().to_owned(), None),
            |t| {
                let r = self.render_for(
                    t,
                    &path,
                    number.map_or(Number::None, Number::Value),
                    req.date,
                    WeekdayBase::LocaleWeek,
                    None,
                    &req.prompts,
                );
                if !r.invalid.is_empty() {
                    warning = Some(format!(
                        "The template has tokens that did not write: {}",
                        r.invalid.join(" ")
                    ));
                }
                (r.content, r.cursor)
            },
        );
        Ok(Made::Done {
            out: Outcome::Write {
                ops: vec![FileOp::CreateFile {
                    path: path.clone(),
                    text: content,
                }],
                open: Some((path, req.how)),
            },
            cursor,
            warning,
        })
    }

    /// The request the "create note" command `which` (its id, name or
    /// number) makes (`runTemplateCommand`): its template, its name
    /// format, its folder.
    ///
    /// # Errors
    ///
    /// No such command, or its folder is gone.
    pub fn template_command_request(&self, which: &str, how: OpenIn) -> Result<NewNote, String> {
        let c = self
            .templates
            .command(which)
            .ok_or_else(|| format!("no create-note command {which:?}"))?;
        let folder = match c.location {
            CommandLocation::Current => self.current_folder(),
            CommandLocation::Folder => {
                let f = c.folder.trim().trim_matches('/').to_owned();
                if !f.is_empty() && !self.all.has_folder(&f) {
                    return Err(format!("The folder {} is not there", c.folder));
                }
                f
            }
        };
        Ok(NewNote {
            folder,
            template: Some(c.template.clone()).filter(|t| !t.trim().is_empty()),
            no_template: c.template.trim().is_empty(),
            name_format: Some(c.name_format.clone()).filter(|f| !f.trim().is_empty()),
            how,
            ..NewNote::default()
        })
    }

    /// Runs the "create note" command `which` with the prompts answered.
    ///
    /// # Errors
    ///
    /// As [`template_command_request`](Self::template_command_request) and
    /// [`new_note`](Self::new_note).
    pub fn run_template_command(
        &self,
        which: &str,
        prompts: BTreeMap<String, String>,
        how: OpenIn,
    ) -> Result<Made, String> {
        let mut req = self.template_command_request(which, how)?;
        req.prompts = prompts;
        self.new_note(&req)
    }

    /// A periodic note's text (`createCalendarNote`): its template's,
    /// written for its first day, `{{date}}` in its file name's pattern so
    /// its links resolve; a heading when it has none.
    pub(super) fn periodic_text(&self, kind: PeriodKind, path: &str, first: NaiveDate) -> String {
        let setting = match kind {
            PeriodKind::Day => &self.settings.daily_template,
            PeriodKind::Week => &self.settings.weekly_template,
            PeriodKind::Month => &self.settings.monthly_template,
            PeriodKind::Quarter => &self.settings.quarterly_template,
            PeriodKind::Year => &self.settings.yearly_template,
        };
        let Some(text) = Some(setting.trim())
            .filter(|t| !t.is_empty())
            .and_then(|t| self.reader.read(t))
            .map(|t| self.engine_text(t))
        else {
            return self.periodic.new_note_text(path);
        };
        let name_pattern = self
            .periodic
            .file_pattern(kind)
            .map(|p| p.source().to_owned());
        let base = if kind == PeriodKind::Week {
            WeekdayBase::NoteDate
        } else {
            WeekdayBase::LocaleWeek
        };
        self.render_for(
            &text,
            path,
            Number::None,
            Some(first),
            base,
            name_pattern.as_deref(),
            &BTreeMap::new(),
        )
        .content
    }

    /// A new folder in `parent` (`createNewFolder`).
    ///
    /// # Errors
    ///
    /// No such parent, a name that cannot be, or it is taken.
    pub fn new_folder(&self, parent_folder: &str, name: &str) -> Result<Outcome, String> {
        let up = parent_folder.trim_matches('/');
        if !up.is_empty() && !self.all.has_folder(up) {
            return Err(format!("no folder {up}"));
        }
        let check = check_name(name);
        if let Some(e) = check.error {
            return Err(e);
        }
        let path = join(up, name.trim());
        if self.taken_any_case(&path) {
            return Err(format!("{path} is there already"));
        }
        Ok(Outcome::Write {
            ops: vec![FileOp::CreateFolder(path)],
            open: None,
        })
    }

    /// Renames the file at `path` to `name` (`renameFileToName`): its
    /// extension kept (typed or not); a note's links follow.
    ///
    /// # Errors
    ///
    /// No such file, a name that cannot be, or it is taken.
    pub fn rename_file(&self, path: &str, name: &str) -> Result<Outcome, String> {
        let f = self
            .all
            .file(path)
            .ok_or_else(|| format!("no file {path}"))?;
        let check = check_name(name);
        if let Some(e) = check.error {
            return Err(e);
        }
        let ext = f.extension().to_owned();
        let name = name.trim();
        let stem = if ext.is_empty() {
            name
        } else {
            name.strip_suffix(&format!(".{ext}"))
                .or_else(|| {
                    let lower = name.to_lowercase();
                    lower
                        .ends_with(&format!(".{}", ext.to_lowercase()))
                        .then(|| name.get(..name.len().saturating_sub(ext.len().saturating_add(1))))
                        .flatten()
                })
                .unwrap_or(name)
        }
        .trim_end_matches('.');
        if stem.is_empty() {
            return Err("A name is needed".to_owned());
        }
        let to = in_folder(f.folder(), stem, &ext);
        if to == path {
            return Err("It has that name already".to_owned());
        }
        if !to.eq_ignore_ascii_case(path) && self.taken_any_case(&to) {
            return Err(format!("{to} is there already"));
        }
        Ok(Outcome::Write {
            ops: vec![FileOp::Move {
                from: path.to_owned(),
                to,
            }],
            open: None,
        })
    }

    /// Renames the folder `path` to `name` (`renameFolderToName`); its
    /// folder note, when named after it, is renamed with it.
    ///
    /// # Errors
    ///
    /// The root, no such folder, a name that cannot be, it is taken, or
    /// its folder note's new name is.
    pub fn rename_folder(&self, path: &str, name: &str) -> Result<Outcome, String> {
        let path = path.trim_matches('/');
        if path.is_empty() {
            return Err("The vault's root is renamed with the vault".to_owned());
        }
        if !self.all.has_folder(path) {
            return Err(format!("no folder {path}"));
        }
        let check = check_name(name);
        if let Some(e) = check.error {
            return Err(e);
        }
        let to = join(parent(path), name.trim());
        if to == path {
            return Err("It has that name already".to_owned());
        }
        if !to.eq_ignore_ascii_case(path) && self.taken_any_case(&to) {
            return Err(format!("{to} is there already"));
        }
        let mut ops = vec![FileOp::MoveFolder {
            from: path.to_owned(),
            to: to.clone(),
        }];
        if self.settings.folder_notes
            && self.settings.folder_note_name.contains("{{folder}}")
            && let Some(note) = self.folder_note(path)
        {
            let ext = self
                .all
                .file(note)
                .map_or("md", crate::source::NoteFile::extension)
                .to_owned();
            let file = note.rsplit('/').next().unwrap_or(note);
            let new_name = format!(
                "{}.{ext}",
                crate::list::folder_note_name(&self.settings, &to)
            );
            let at = join(&to, file);
            let new_path = join(&to, &new_name);
            if self.taken(&join(path, &new_name)) && new_name != file {
                return Err(format!("A note {new_name} is in the folder already"));
            }
            if at != new_path {
                ops.push(FileOp::Move {
                    from: at,
                    to: new_path,
                });
            }
        }
        Ok(Outcome::Write { ops, open: None })
    }

    /// Moves the folder `path` into `into` (a drop; `moveFolderToTarget`).
    ///
    /// # Errors
    ///
    /// The root, into itself or where it is already, or the name is taken
    /// there.
    pub fn move_folder(&self, path: &str, into: &str) -> Result<Outcome, String> {
        let (path, into) = (path.trim_matches('/'), into.trim_matches('/'));
        if path.is_empty() {
            return Err("The vault's root does not move".to_owned());
        }
        if !self.all.has_folder(path) {
            return Err(format!("no folder {path}"));
        }
        if !into.is_empty() && !self.all.has_folder(into) {
            return Err(format!("no folder {into}"));
        }
        if into == path || is_within(into, path) {
            return Err(format!("{path} cannot go into itself"));
        }
        if parent(path) == into {
            return Err(format!("{path} is there already"));
        }
        let name = path.rsplit('/').next().unwrap_or(path);
        let to = join(into, name);
        if self.taken_any_case(&to) {
            return Err(format!("{to} is there already"));
        }
        Ok(Outcome::Write {
            ops: vec![FileOp::MoveFolder {
                from: path.to_owned(),
                to,
            }],
            open: None,
        })
    }

    /// Moves files into `folder` (a drop, "Move to…"; `moveFilesToFolder`).
    /// Files there already are left.
    ///
    /// # Errors
    ///
    /// No such folder, nothing to move, or a name is taken there.
    pub fn move_files(&self, paths: &[String], folder: &str) -> Result<Outcome, String> {
        let folder = folder.trim_matches('/');
        if !folder.is_empty() && !self.all.has_folder(folder) {
            return Err(format!("no folder {folder}"));
        }
        let mut ops = Vec::new();
        let mut seen = BTreeSet::new();
        for p in paths {
            let f = self.all.file(p).ok_or_else(|| format!("no file {p}"))?;
            if f.folder() == folder {
                continue;
            }
            let to = join(folder, f.file_name());
            if self.taken_any_case(&to) || !seen.insert(to.to_lowercase()) {
                return Err(format!("{to} is there already"));
            }
            ops.push(FileOp::Move {
                from: p.clone(),
                to,
            });
        }
        if ops.is_empty() {
            return Err(format!(
                "{} in {} already",
                if paths.len() == 1 {
                    "It is"
                } else {
                    "They are"
                },
                if folder.is_empty() {
                    "the root"
                } else {
                    folder
                }
            ));
        }
        Ok(Outcome::Write { ops, open: None })
    }

    /// Copies files into `folder` (a drop with the copy key held;
    /// `copyFilesToFolder`): each keeps its name, numbered when taken.
    ///
    /// # Errors
    ///
    /// No such folder, or nothing to copy.
    pub fn copy_files(&self, paths: &[String], folder: &str) -> Result<Outcome, String> {
        let folder = folder.trim_matches('/');
        if !folder.is_empty() && !self.all.has_folder(folder) {
            return Err(format!("no folder {folder}"));
        }
        let mut ops = Vec::new();
        let mut claimed: BTreeSet<String> = BTreeSet::new();
        for p in paths {
            let f = self.all.file(p).ok_or_else(|| format!("no file {p}"))?;
            let ext = f.extension().to_owned();
            let name = unique_name(folder, f.name(), &ext, |c| {
                self.taken_any_case(c) || claimed.contains(&c.to_lowercase())
            });
            let to = in_folder(folder, &name, &ext);
            claimed.insert(to.to_lowercase());
            ops.push(FileOp::Copy {
                from: p.clone(),
                to,
            });
        }
        if ops.is_empty() {
            return Err("Nothing to copy".to_owned());
        }
        Ok(Outcome::Write { ops, open: None })
    }

    /// Duplicates the file at `path` beside it (`duplicateNote`: `Plan 1`)
    /// and opens the copy.
    ///
    /// # Errors
    ///
    /// No such file.
    pub fn duplicate(&self, path: &str) -> Result<Outcome, String> {
        let f = self
            .all
            .file(path)
            .ok_or_else(|| format!("no file {path}"))?;
        let ext = f.extension().to_owned();
        let name = unique_name(f.folder(), f.name(), &ext, |c| self.taken_any_case(c));
        let to = in_folder(f.folder(), &name, &ext);
        Ok(Outcome::Write {
            ops: vec![FileOp::Copy {
                from: path.to_owned(),
                to: to.clone(),
            }],
            open: Some((to, OpenIn::Here)),
        })
    }

    /// Duplicates the folder `path` beside it (`duplicateFolder`:
    /// `Projects 1`, `Projects 2`, …).
    ///
    /// # Errors
    ///
    /// The root, or no such folder.
    pub fn duplicate_folder(&self, path: &str) -> Result<Outcome, String> {
        let path = path.trim_matches('/');
        if path.is_empty() {
            return Err("The vault's root cannot be duplicated".to_owned());
        }
        if !self.all.has_folder(path) {
            return Err(format!("no folder {path}"));
        }
        let to = (1..=u32::MAX)
            .map(|n| format!("{path} {n}"))
            .find(|p| !self.taken_any_case(p))
            .unwrap_or_default();
        Ok(Outcome::Write {
            ops: vec![FileOp::CopyFolder {
                from: path.to_owned(),
                to,
            }],
            open: None,
        })
    }

    /// Puts files or folders in the trash (`deleteMultipleFiles`,
    /// `deleteFolder`); a folder takes what is in it. The view asks first
    /// when `confirm_delete` is on.
    ///
    /// # Errors
    ///
    /// Nothing to trash, the root, or something is not there.
    pub fn trash(&self, paths: &[String]) -> Result<Outcome, String> {
        let mut ops = Vec::new();
        let mut gone: Vec<&str> = Vec::new();
        for p in paths {
            let p = p.trim_matches('/');
            if p.is_empty() {
                return Err("The vault's root cannot go to the trash".to_owned());
            }
            if !self.taken(p) {
                return Err(format!("nothing at {p}"));
            }
            // Something inside a folder that goes too goes with it.
            if gone.iter().any(|g| is_within(p, g)) {
                continue;
            }
            gone.retain(|g| !is_within(g, p));
            gone.push(p);
        }
        for g in gone {
            ops.push(FileOp::Trash(g.to_owned()));
        }
        if ops.is_empty() {
            return Err("Nothing to delete".to_owned());
        }
        Ok(Outcome::Write { ops, open: None })
    }

    /// What the person is asked before [`trash`](Self::trash): `Delete
    /// "Plan"?`, `Delete 3 items?`, a folder with its contents.
    #[must_use]
    pub fn trash_question(&self, paths: &[String]) -> String {
        match paths {
            [one] => {
                let name = one.rsplit('/').next().unwrap_or(one);
                if self.all.has_folder(one) {
                    let n = self.all.files().filter(|f| is_within(&f.path, one)).count();
                    format!(
                        "Delete the folder \"{name}\" and the {n} files in it? It goes to the trash."
                    )
                } else {
                    format!("Delete \"{name}\"? It goes to the trash.")
                }
            }
            many => format!("Delete {} items? They go to the trash.", many.len()),
        }
    }

    /// Merges Markdown notes into one (`mergeNotes`): the first's
    /// frontmatter kept, `sep` between, in the order given; made in
    /// `folder` (the first's when none) as `name` (numbered when taken),
    /// opened; the notes to the trash after when `trash_sources`.
    ///
    /// # Errors
    ///
    /// Fewer than two notes, no name, a name that cannot be, or a note's
    /// text cannot be read.
    pub fn merge(
        &self,
        paths: &[String],
        name: &str,
        folder: Option<&str>,
        sep: MergeSeparator,
        trash_sources: bool,
    ) -> Result<Outcome, String> {
        let mut seen = BTreeSet::new();
        let notes: Vec<&crate::source::NoteFile> = paths
            .iter()
            .filter_map(|p| self.all.file(p))
            .filter(|f| f.is_markdown() && seen.insert(f.path.clone()))
            .collect();
        if notes.len() < 2 {
            return Err("Merging takes two Markdown notes or more".to_owned());
        }
        let name = name.trim();
        let name = name.strip_suffix(".md").unwrap_or(name).trim();
        let name = name.trim_start_matches('.').replace([':', '/'], "");
        let name = name.trim();
        if name.is_empty() {
            return Err("The merged note needs a name".to_owned());
        }
        let folder = folder.map_or_else(
            || {
                notes
                    .first()
                    .map_or_else(String::new, |f| f.folder().to_owned())
            },
            |f| f.trim_matches('/').to_owned(),
        );
        if !folder.is_empty() && !self.all.has_folder(&folder) {
            return Err(format!("no folder {folder}"));
        }
        let mut texts = Vec::new();
        for f in &notes {
            let text = self
                .reader
                .read(&f.path)
                .ok_or_else(|| format!("{} could not be read", f.path))?;
            texts.push((f.name().to_owned(), text));
        }
        let base = unique_name(&folder, name, "md", |p| self.taken_any_case(p));
        let path = in_folder(&folder, &base, "md");
        let mut ops = vec![FileOp::CreateFile {
            path: path.clone(),
            text: merged_text(&texts, sep),
        }];
        if trash_sources {
            ops.extend(notes.iter().map(|f| FileOp::Trash(f.path.clone())));
        }
        Ok(Outcome::Write {
            ops,
            open: Some((path, OpenIn::Here)),
        })
    }

    /// The notes "Open all files" opens (`openAllFilesInCurrentFolderOrTag`):
    /// the list's, in its order. The view asks first at
    /// [`OPEN_ALL_WARNING`] or more.
    ///
    /// # Errors
    ///
    /// The list is empty.
    pub fn open_all(&self) -> Result<Vec<String>, String> {
        let all: Vec<String> = self.list_order().into_iter().map(str::to_owned).collect();
        if all.is_empty() {
            return Err("No notes here".to_owned());
        }
        Ok(all)
    }

    /// The notes selected, in the list's order (the menu's "Open N in new
    /// tabs").
    #[must_use]
    pub fn selected_in_order(&self) -> Vec<String> {
        let picked = self.selection.files();
        self.list_order()
            .into_iter()
            .filter(|p| picked.contains(*p))
            .map(str::to_owned)
            .collect()
    }

    /// What a drop on a folder does (`useDragAndDrop`'s folder case):
    /// notes move into it (copied with the copy key: ⌥ on a Mac, Ctrl
    /// elsewhere); a folder moves into it.
    pub(super) fn drop_on_folder(
        &self,
        drag: &Drag,
        folder: &str,
        copy: bool,
    ) -> Result<Outcome, String> {
        match drag {
            Drag::Notes(paths) if copy => self.copy_files(paths, folder),
            Drag::Notes(paths) => self.move_files(paths, folder),
            Drag::Folder(src) => self.move_folder(src, folder),
            Drag::Tag(t) => Err(format!("#{t} goes on a tag")),
            Drag::Shortcut(_) => Err("A shortcut goes among the shortcuts".to_owned()),
        }
    }

    /// The folder `path` opened under a drag held over it (spring-loaded
    /// folders); whether it opened.
    pub fn spring_open(&mut self, target: &NavTarget) -> bool {
        match target {
            NavTarget::Folder(path) => {
                let row = self
                    .nav_rows
                    .iter()
                    .find(|r| r.folder() == Some(path.as_str()))
                    .and_then(crate::tree::NavRow::expansion);
                if row == Some(false) {
                    self.set_expanded(path, true);
                    true
                } else {
                    false
                }
            }
            // A tag, collapsed with tags below it, opens as a folder does.
            NavTarget::Tag(tag) => {
                let collapsed = self.nav_rows.iter().any(|r| {
                    matches!(&r.kind, crate::tree::NavRowKind::Tag { tag: t, has_children: true, expanded: false, .. } if crate::tags::tag_key(t) == crate::tags::tag_key(tag))
                });
                if collapsed {
                    self.set_nav_expanded(target, true);
                }
                collapsed
            }
            NavTarget::Property { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_checked() {
        assert!(check_name("Plan").ok());
        assert!(!check_name("").ok());
        assert!(!check_name(".hidden").ok());
        assert!(!check_name("a:b").ok());
        assert!(!check_name("a/b").ok());
        let w = check_name("C# notes");
        assert!(w.ok());
        assert!(w.warning.is_some());
        assert!(check_name("a [[b]]").warning.is_some());
    }

    #[test]
    fn merged_text_keeps_the_first_frontmatter() {
        let notes = vec![
            ("A".to_owned(), "---\ntags: [a]\n---\nOne\n\n".to_owned()),
            ("B".to_owned(), "---\nx: 1\n---\n\nTwo\n".to_owned()),
            ("C".to_owned(), "Three".to_owned()),
        ];
        assert_eq!(
            merged_text(&notes, MergeSeparator::BlankLine),
            "---\ntags: [a]\n---\nOne\n\nTwo\n\nThree\n"
        );
        assert_eq!(
            merged_text(&notes, MergeSeparator::Heading),
            "---\ntags: [a]\n---\nOne\n\n# B\n\nTwo\n\n# C\n\nThree\n"
        );
        assert_eq!(
            merged_text(&notes[1..], MergeSeparator::None),
            "---\nx: 1\n---\n\nTwo\nThree\n"
        );
    }
}
