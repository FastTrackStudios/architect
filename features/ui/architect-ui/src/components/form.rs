//! Form — TanStack-inspired form state and validation for Dioxus.

use std::collections::HashMap;

use architect_story_runtime::story;
use dioxus::prelude::*;

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct FormFieldState {
    pub value: String,
    pub initial_value: String,
    pub touched: bool,
    pub dirty: bool,
    pub errors: Vec<String>,
}

impl FormFieldState {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        Self {
            value: value.clone(),
            initial_value: value,
            touched: false,
            dirty: false,
            errors: Vec::new(),
        }
    }

    pub fn valid(&self) -> bool {
        self.errors.is_empty()
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct FormState {
    pub fields: HashMap<String, FormFieldState>,
    pub submit_count: usize,
    pub submitting: bool,
}

impl FormState {
    pub fn valid(&self) -> bool {
        self.fields.values().all(FormFieldState::valid)
    }

    pub fn dirty(&self) -> bool {
        self.fields.values().any(|field| field.dirty)
    }

    pub fn touched(&self) -> bool {
        self.fields.values().any(|field| field.touched)
    }

    pub fn values(&self) -> HashMap<String, String> {
        self.fields
            .iter()
            .map(|(name, field)| (name.clone(), field.value.clone()))
            .collect()
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FormRule {
    Required { message: String },
    MinLength { min: usize, message: String },
    MaxLength { max: usize, message: String },
    Email { message: String },
    Matches { expected: String, message: String },
}

impl FormRule {
    pub fn required(message: impl Into<String>) -> Self {
        Self::Required {
            message: message.into(),
        }
    }

    pub fn min_length(min: usize, message: impl Into<String>) -> Self {
        Self::MinLength {
            min,
            message: message.into(),
        }
    }

    pub fn max_length(max: usize, message: impl Into<String>) -> Self {
        Self::MaxLength {
            max,
            message: message.into(),
        }
    }

    pub fn email(message: impl Into<String>) -> Self {
        Self::Email {
            message: message.into(),
        }
    }

    pub fn matches(expected: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Matches {
            expected: expected.into(),
            message: message.into(),
        }
    }

    fn validate(&self, value: &str) -> Option<String> {
        match self {
            Self::Required { message } if value.trim().is_empty() => Some(message.clone()),
            Self::MinLength { min, message } if value.chars().count() < *min => {
                Some(message.clone())
            }
            Self::MaxLength { max, message } if value.chars().count() > *max => {
                Some(message.clone())
            }
            Self::Email { message } if !looks_like_email(value) => Some(message.clone()),
            Self::Matches { expected, message } if value != expected => Some(message.clone()),
            _ => None,
        }
    }
}

fn looks_like_email(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };

    !local.is_empty() && domain.contains('.') && !domain.ends_with('.')
}

#[derive(Clone, Copy, PartialEq)]
pub struct FormApi {
    state: Signal<FormState>,
}

pub fn use_form() -> FormApi {
    FormApi {
        state: use_signal(FormState::default),
    }
}

impl FormApi {
    pub fn state(&self) -> FormState {
        (self.state)()
    }

    pub fn values(&self) -> HashMap<String, String> {
        self.state().values()
    }

    pub fn register(&mut self, name: impl Into<String>, initial_value: impl Into<String>) {
        let name = name.into();
        let initial_value = initial_value.into();
        self.state.with_mut(|state| {
            state
                .fields
                .entry(name)
                .or_insert_with(|| FormFieldState::new(initial_value));
        });
    }

    pub fn field(&self, name: impl Into<String>) -> FieldApi {
        FieldApi {
            name: name.into(),
            form: *self,
        }
    }

    pub fn value(&self, name: &str) -> String {
        self.state()
            .fields
            .get(name)
            .map(|field| field.value.clone())
            .unwrap_or_default()
    }

    pub fn set_value(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        let value = value.into();
        self.state.with_mut(|state| {
            let field = state
                .fields
                .entry(name)
                .or_insert_with(|| FormFieldState::new(""));
            field.value = value;
            field.dirty = field.value != field.initial_value;
        });
    }

    pub fn mark_touched(&mut self, name: impl Into<String>) {
        let name = name.into();
        self.state.with_mut(|state| {
            state
                .fields
                .entry(name)
                .or_insert_with(|| FormFieldState::new(""))
                .touched = true;
        });
    }

    pub fn validate_field(&mut self, name: impl Into<String>, rules: &[FormRule]) -> bool {
        let name = name.into();
        self.state.with_mut(|state| {
            let field = state
                .fields
                .entry(name)
                .or_insert_with(|| FormFieldState::new(""));
            field.errors = rules
                .iter()
                .filter_map(|rule| rule.validate(&field.value))
                .collect();
            field.errors.is_empty()
        })
    }

    pub fn set_errors(&mut self, name: impl Into<String>, errors: Vec<String>) {
        let name = name.into();
        self.state.with_mut(|state| {
            state
                .fields
                .entry(name)
                .or_insert_with(|| FormFieldState::new(""))
                .errors = errors;
        });
    }

    pub fn reset(&mut self) {
        self.state.with_mut(|state| {
            state.submit_count = 0;
            state.submitting = false;
            for field in state.fields.values_mut() {
                field.value = field.initial_value.clone();
                field.dirty = false;
                field.touched = false;
                field.errors.clear();
            }
        });
    }

    pub fn submit(&mut self) {
        self.state.with_mut(|state| {
            state.submit_count += 1;
            for field in state.fields.values_mut() {
                field.touched = true;
            }
        });
    }

    pub fn set_submitting(&mut self, submitting: bool) {
        self.state.write().submitting = submitting;
    }
}

#[derive(Clone, PartialEq)]
pub struct FieldApi {
    name: String,
    form: FormApi,
}

impl FieldApi {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn state(&self) -> FormFieldState {
        self.form
            .state()
            .fields
            .get(&self.name)
            .cloned()
            .unwrap_or_default()
    }

    pub fn value(&self) -> String {
        self.state().value
    }

    pub fn error(&self) -> Option<String> {
        self.state().errors.first().cloned()
    }

    pub fn oninput(&self) -> Callback<FormEvent> {
        let mut form = self.form;
        let name = self.name.clone();
        Callback::new(move |event: FormEvent| {
            form.set_value(name.clone(), event.value());
        })
    }

    pub fn onblur(&self) -> Callback<FocusEvent> {
        let mut form = self.form;
        let name = self.name.clone();
        Callback::new(move |_| {
            form.mark_touched(name.clone());
        })
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct FormProps {
    #[props(default)]
    pub on_submit: Option<Callback<FormEvent>>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn Form(props: FormProps) -> Element {
    rsx! {
        form {
            class: crate::cn::merge_slice(&["grid gap-4", props.class.as_str()]),
            onsubmit: move |event| {
                event.prevent_default();
                if let Some(callback) = &props.on_submit {
                    callback.call(event);
                }
            },
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct FormMessageProps {
    #[props(default)]
    pub error: Option<String>,
    #[props(default)]
    pub class: String,
}

#[component]
pub fn FormMessage(props: FormMessageProps) -> Element {
    rsx! {
        if let Some(error) = props.error {
            p {
                class: crate::cn::merge_slice(&["text-sm text-destructive", props.class.as_str()]),
                "{error}"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_rule_rejects_blank_values() {
        let rule = FormRule::required("Required");

        assert_eq!(rule.validate(""), Some("Required".to_string()));
        assert_eq!(rule.validate("value"), None);
    }

    #[test]
    fn form_state_reports_values_and_dirty_status() {
        let mut state = FormState::default();
        state
            .fields
            .insert("name".into(), FormFieldState::new("Ada"));
        let field = state.fields.get_mut("name").unwrap();
        field.value = "Grace".into();
        field.dirty = field.value != field.initial_value;

        assert!(state.dirty());
        assert_eq!(state.values().get("name"), Some(&"Grace".to_string()));
    }
}

#[story(category = "Form", name = "form_default")]
pub fn form_default() -> Element {
    let mut form = use_form();
    let mut initialized = use_signal(|| false);
    if !initialized() {
        form.register("name", "Ada Lovelace");
        form.register("email", "ada@example.com");
        initialized.set(true);
    }
    let name = form.field("name");
    let email = form.field("email");
    let state = form.state();
    let submitted = state.submit_count > 0;
    let name_error = if submitted && name.value().trim().is_empty() {
        Some("Name is required.".to_string())
    } else {
        None
    };
    let email_error = if submitted && !email.value().contains('@') {
        Some("Enter a valid email address.".to_string())
    } else {
        None
    };

    rsx! {
        div { class: "p-6 bg-background text-foreground",
            Form {
                class: "max-w-xl".to_string(),
                on_submit: move |_| {
                    form.submit();
                    form.validate_field("name", &[FormRule::required("Name is required.")]);
                    form.validate_field("email", &[FormRule::email("Enter a valid email address.")]);
                },
                crate::components::Field {
                    crate::components::FieldLabel { required: true, "Name" }
                    input {
                        class: "h-9 w-full rounded-lg border border-input bg-transparent px-3 py-1 text-sm",
                        value: "{name.value()}",
                        oninput: name.oninput(),
                        onblur: name.onblur(),
                    }
                    FormMessage { error: name_error }
                }
                crate::components::Field {
                    crate::components::FieldLabel { required: true, "Email" }
                    input {
                        class: "h-9 w-full rounded-lg border border-input bg-transparent px-3 py-1 text-sm",
                        value: "{email.value()}",
                        oninput: email.oninput(),
                        onblur: email.onblur(),
                    }
                    FormMessage { error: email_error }
                }
                crate::components::Button { variant: crate::components::ButtonVariant::Primary, "Validate" }
            }
        }
    }
}
