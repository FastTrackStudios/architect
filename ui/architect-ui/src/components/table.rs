//! Table — shadcn v4 maia style.

use dioxus::prelude::*;
use architect_story_runtime::story;

#[derive(Props, Clone, PartialEq)]
pub struct TableContainerProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-table-container
#[component]
pub fn TableContainer(props: TableContainerProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["relative w-full overflow-x-auto", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct TableProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-table
#[component]
pub fn Table(props: TableProps) -> Element {
    rsx! {
        table {
            class: crate::cn::merge_slice(&["w-full caption-bottom text-sm", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct TableHeaderProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-table-header
#[component]
pub fn TableHeader(props: TableHeaderProps) -> Element {
    rsx! {
        thead {
            class: crate::cn::merge_slice(&["[&_tr]:border-b [&_tr]:border-border", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct TableBodyProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-table-body
#[component]
pub fn TableBody(props: TableBodyProps) -> Element {
    rsx! {
        tbody {
            class: crate::cn::merge_slice(&["[&_tr:last-child]:border-0", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct TableFooterProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-table-footer
#[component]
pub fn TableFooter(props: TableFooterProps) -> Element {
    rsx! {
        tfoot {
            class: crate::cn::merge(format!(
                "bg-muted/50 border-t border-border font-medium [&>tr]:last:border-b-0 {}",
                props.class
            )),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct TableRowProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-table-row
#[component]
pub fn TableRow(props: TableRowProps) -> Element {
    rsx! {
        tr {
            class: crate::cn::merge(format!(
                "hover:bg-muted/50 data-[state=selected]:bg-muted border-b border-border transition-colors {}",
                props.class
            )),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct TableHeadProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-table-head
#[component]
pub fn TableHead(props: TableHeadProps) -> Element {
    rsx! {
        th {
            class: crate::cn::merge(format!(
                "text-foreground h-12 px-3 text-left align-middle font-medium whitespace-nowrap {}",
                props.class
            )),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct TableCellProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-table-cell
#[component]
pub fn TableCell(props: TableCellProps) -> Element {
    rsx! {
        td {
            class: crate::cn::merge_slice(&["p-3 align-middle whitespace-nowrap", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct TableCaptionProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-table-caption
#[component]
pub fn TableCaption(props: TableCaptionProps) -> Element {
    rsx! {
        caption {
            class: crate::cn::merge_slice(&["text-muted-foreground mt-4 text-sm", props.class.as_str()]),
            {props.children}
        }
    }
}

/// Default Table story showing a small task list.
#[story(category = "Table", name = "default")]
pub fn table_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            TableContainer {
                Table {
                    TableHeader {
                        TableRow {
                            TableHead { "Name" }
                            TableHead { "Status" }
                            TableHead { "Due" }
                        }
                    }
                    TableBody {
                        TableRow {
                            TableCell { "Auth bug" }
                            TableCell { "WIP" }
                            TableCell { "Apr 11" }
                        }
                        TableRow {
                            TableCell { "Dashboard" }
                            TableCell { "Open" }
                            TableCell { "Apr 15" }
                        }
                        TableRow {
                            TableCell { "Onboarding" }
                            TableCell { "Done" }
                            TableCell { "Apr 04" }
                        }
                    }
                }
            }
        }
    }
}
