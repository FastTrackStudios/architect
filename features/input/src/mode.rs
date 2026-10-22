//! Modal editing: mode ids, mode definitions and the mode stack.
//!
//! The [`ModeStack`] holds a base mode with transient sub-modes pushed on
//! top. Every transition returns the `on_exit` / `on_enter` actions of the
//! modes it leaves and enters, so the app can react to mode changes.

use std::collections::HashMap;
use std::fmt;

use crate::command::InputCommand;

/// An editing mode's id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModeId(pub String);

impl ModeId {
    pub const NORMAL: &str = "normal";
    pub const INSERT: &str = "insert";
    pub const VISUAL: &str = "visual";
    pub const COMMAND: &str = "command";

    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn normal() -> Self {
        Self::new(Self::NORMAL)
    }

    #[must_use]
    pub fn insert() -> Self {
        Self::new(Self::INSERT)
    }

    #[must_use]
    pub fn visual() -> Self {
        Self::new(Self::VISUAL)
    }

    #[must_use]
    pub fn command() -> Self {
        Self::new(Self::COMMAND)
    }
}

impl fmt::Display for ModeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How a mode behaves, and what runs when it is entered or left.
#[derive(Debug, Clone)]
pub struct ModeDefinition<A = String> {
    pub id: ModeId,
    /// What a status line shows (`NORMAL`, `INSERT`).
    pub display_name: String,
    /// Unbound plain character keys become [`InputCommand::InsertText`].
    pub passthrough_text: bool,
    /// The mode persists after an action (sub-mode behaviour).
    pub sticky: bool,
    /// Actions run when the mode is entered.
    pub on_enter: Vec<A>,
    /// Actions run when the mode is left.
    pub on_exit: Vec<A>,
}

impl<A> ModeDefinition<A> {
    #[must_use]
    pub fn new(id: ModeId, display_name: impl Into<String>) -> Self {
        Self {
            id,
            display_name: display_name.into(),
            passthrough_text: false,
            sticky: false,
            on_enter: Vec::new(),
            on_exit: Vec::new(),
        }
    }

    #[must_use]
    pub const fn with_passthrough_text(mut self, passthrough: bool) -> Self {
        self.passthrough_text = passthrough;
        self
    }

    #[must_use]
    pub const fn with_sticky(mut self, sticky: bool) -> Self {
        self.sticky = sticky;
        self
    }

    #[must_use]
    pub fn with_on_enter(mut self, actions: Vec<A>) -> Self {
        self.on_enter = actions;
        self
    }

    #[must_use]
    pub fn with_on_exit(mut self, actions: Vec<A>) -> Self {
        self.on_exit = actions;
        self
    }
}

/// A base mode with sub-modes stacked on top of it.
///
/// The base is its own field, so the stack can never be empty.
#[derive(Debug, Clone)]
pub struct ModeStack<A = String> {
    base: ModeId,
    subs: Vec<ModeId>,
    definitions: HashMap<ModeId, ModeDefinition<A>>,
}

impl<A: Clone> ModeStack<A> {
    /// A stack holding only `initial`.
    #[must_use]
    pub fn new(initial: ModeId) -> Self {
        Self {
            base: initial,
            subs: Vec::new(),
            definitions: HashMap::new(),
        }
    }

    /// Register (or replace) a mode definition.
    pub fn add_mode(&mut self, def: ModeDefinition<A>) {
        self.definitions.insert(def.id.clone(), def);
    }

    /// The topmost mode.
    #[must_use]
    pub fn current(&self) -> &ModeId {
        self.subs.last().unwrap_or(&self.base)
    }

    /// The base (bottom) mode.
    #[must_use]
    pub const fn base(&self) -> &ModeId {
        &self.base
    }

    /// How many modes are stacked, the base included.
    #[must_use]
    pub const fn depth(&self) -> usize {
        self.subs.len().saturating_add(1)
    }

    /// Switch the base mode, dropping every sub-mode: the `on_exit`
    /// actions of each mode left (top first), then the new base's
    /// `on_enter`.
    pub fn switch_base(&mut self, new_base: ModeId) -> Vec<InputCommand<A>> {
        let entering = self.enter_actions(&new_base);
        let old_base = std::mem::replace(&mut self.base, new_base);
        let left: Vec<ModeId> = self
            .subs
            .drain(..)
            .rev()
            .chain(std::iter::once(old_base))
            .collect();
        let mut commands: Vec<InputCommand<A>> = left
            .iter()
            .flat_map(|mode| self.exit_actions(mode))
            .collect();
        commands.extend(entering);
        commands
    }

    /// Push a sub-mode: its `on_enter` actions.
    pub fn push(&mut self, mode: ModeId) -> Vec<InputCommand<A>> {
        let commands = self.enter_actions(&mode);
        self.subs.push(mode);
        commands
    }

    /// Pop the top sub-mode: its `on_exit` actions. Nothing when only the
    /// base is left.
    pub fn pop(&mut self) -> Vec<InputCommand<A>> {
        self.subs
            .pop()
            .map(|popped| self.exit_actions(&popped))
            .unwrap_or_default()
    }

    /// A mode's definition, if registered.
    #[must_use]
    pub fn definition(&self, id: &ModeId) -> Option<&ModeDefinition<A>> {
        self.definitions.get(id)
    }

    /// The current mode's definition, if registered.
    #[must_use]
    pub fn current_definition(&self) -> Option<&ModeDefinition<A>> {
        self.definitions.get(self.current())
    }

    fn enter_actions(&self, mode: &ModeId) -> Vec<InputCommand<A>> {
        self.actions_of(mode, |def| &def.on_enter)
    }

    fn exit_actions(&self, mode: &ModeId) -> Vec<InputCommand<A>> {
        self.actions_of(mode, |def| &def.on_exit)
    }

    fn actions_of(
        &self,
        mode: &ModeId,
        pick: impl Fn(&ModeDefinition<A>) -> &Vec<A>,
    ) -> Vec<InputCommand<A>> {
        self.definitions
            .get(mode)
            .map(|def| {
                pick(def)
                    .iter()
                    .cloned()
                    .map(InputCommand::Action)
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: ModeId, name: &str) -> ModeDefinition {
        let tag = id.as_str().to_owned();
        ModeDefinition::new(id, name)
            .with_on_enter(vec![format!("mode.{tag}.enter")])
            .with_on_exit(vec![format!("mode.{tag}.exit")])
    }

    fn extract_action_ids(commands: &[InputCommand]) -> Vec<&str> {
        commands
            .iter()
            .filter_map(|cmd| cmd.action().map(String::as_str))
            .collect()
    }

    fn make_stack_with_modes() -> ModeStack {
        let mut stack = ModeStack::new(ModeId::normal());
        stack.add_mode(def(ModeId::normal(), "NORMAL"));
        stack.add_mode(def(ModeId::insert(), "INSERT").with_passthrough_text(true));
        stack.add_mode(def(ModeId::visual(), "VISUAL"));
        stack.add_mode(def(ModeId::new("goto"), "GOTO").with_sticky(true));
        stack
    }

    #[test]
    fn test_mode_stack_new_starts_with_initial_mode() {
        let stack: ModeStack = ModeStack::new(ModeId::normal());
        assert_eq!(stack.current(), &ModeId::normal());
        assert_eq!(stack.depth(), 1);
    }

    #[test]
    fn test_mode_stack_switch_base_emits_exit_enter_actions() {
        let mut stack = make_stack_with_modes();
        let commands = stack.switch_base(ModeId::insert());
        assert_eq!(
            extract_action_ids(&commands),
            vec!["mode.normal.exit", "mode.insert.enter"]
        );
        assert_eq!(stack.current(), &ModeId::insert());
        assert_eq!(stack.depth(), 1);
    }

    #[test]
    fn test_mode_stack_push_pop_submode() {
        let mut stack = make_stack_with_modes();
        let push_cmds = stack.push(ModeId::new("goto"));
        assert_eq!(extract_action_ids(&push_cmds), vec!["mode.goto.enter"]);
        assert_eq!(stack.current(), &ModeId::new("goto"));
        assert_eq!(stack.depth(), 2);

        let pop_cmds = stack.pop();
        assert_eq!(extract_action_ids(&pop_cmds), vec!["mode.goto.exit"]);
        assert_eq!(stack.current(), &ModeId::normal());
        assert_eq!(stack.depth(), 1);
    }

    #[test]
    fn test_mode_stack_pop_single_mode_is_noop() {
        let mut stack = make_stack_with_modes();
        assert!(stack.pop().is_empty());
        assert_eq!(stack.current(), &ModeId::normal());
        assert_eq!(stack.depth(), 1);
    }

    #[test]
    fn test_mode_stack_switch_base_clears_submodes() {
        let mut stack = make_stack_with_modes();
        stack.push(ModeId::new("goto"));
        let commands = stack.switch_base(ModeId::insert());
        assert_eq!(
            extract_action_ids(&commands),
            vec!["mode.goto.exit", "mode.normal.exit", "mode.insert.enter"]
        );
        assert_eq!(stack.current(), &ModeId::insert());
        assert_eq!(stack.depth(), 1);
    }

    #[test]
    fn test_mode_stack_add_mode_and_definition_lookup() {
        let stack = make_stack_with_modes();
        let normal_def = stack.definition(&ModeId::normal()).unwrap();
        assert_eq!(normal_def.display_name, "NORMAL");
        assert!(!normal_def.passthrough_text);
        let insert_def = stack.definition(&ModeId::insert()).unwrap();
        assert!(insert_def.passthrough_text);
        assert!(stack.definition(&ModeId::new("goto")).unwrap().sticky);
        assert!(stack.definition(&ModeId::new("nonexistent")).is_none());
    }

    #[test]
    fn test_mode_stack_current_definition() {
        let mut stack = make_stack_with_modes();
        assert_eq!(stack.current_definition().unwrap().display_name, "NORMAL");
        stack.push(ModeId::new("goto"));
        assert_eq!(stack.current_definition().unwrap().display_name, "GOTO");
    }

    #[test]
    fn test_mode_stack_multiple_pushes_and_pops() {
        let mut stack = make_stack_with_modes();
        stack.add_mode(def(ModeId::new("window"), "WINDOW"));
        stack.push(ModeId::new("goto"));
        stack.push(ModeId::new("window"));
        assert_eq!(stack.depth(), 3);
        assert_eq!(stack.current(), &ModeId::new("window"));
        assert_eq!(stack.base(), &ModeId::normal());

        assert_eq!(extract_action_ids(&stack.pop()), vec!["mode.window.exit"]);
        assert_eq!(stack.current(), &ModeId::new("goto"));
        assert_eq!(extract_action_ids(&stack.pop()), vec!["mode.goto.exit"]);
        assert_eq!(stack.current(), &ModeId::normal());
        assert!(stack.pop().is_empty());
    }

    #[test]
    fn test_mode_stack_switch_base_without_definitions() {
        let mut stack: ModeStack = ModeStack::new(ModeId::normal());
        assert!(stack.switch_base(ModeId::insert()).is_empty());
        assert_eq!(stack.current(), &ModeId::insert());
    }

    #[test]
    fn test_mode_id_constants() {
        assert_eq!(ModeId::NORMAL, "normal");
        assert_eq!(ModeId::INSERT, "insert");
        assert_eq!(ModeId::VISUAL, "visual");
        assert_eq!(ModeId::COMMAND, "command");
        assert_eq!(ModeId::normal().as_str(), "normal");
    }
}
