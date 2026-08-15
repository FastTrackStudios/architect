//! Data table — TanStack-inspired headless table state for Dioxus.

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
};

use dioxus::prelude::*;
use architect_story_runtime::story;

use super::{
    Select, SelectContent, SelectItem, Table, TableBody, TableCell, TableContainer, TableHead,
    TableHeader, TableRow,
};

#[derive(Clone)]
pub struct DataTableColumn<T> {
    pub id: &'static str,
    pub header: &'static str,
    pub accessor: fn(&T) -> String,
    pub sort_value: Option<fn(&T) -> DataTableSortValue>,
    pub filter: Option<fn(&T, &str) -> bool>,
    pub cell: Option<fn(&T) -> Element>,
    pub header_render: Option<fn(DataTableHeaderContext) -> Element>,
    pub cell_render: Option<fn(DataTableCellContext<T>) -> Element>,
    pub sortable: bool,
    pub filterable: bool,
    pub hideable: bool,
    pub class: &'static str,
    pub head_class: &'static str,
    pub cell_class: &'static str,
    pub dynamic_head_class: Option<fn(DataTableHeaderContext) -> String>,
    pub dynamic_cell_class: Option<fn(DataTableCellContext<T>) -> String>,
}

impl<T> PartialEq for DataTableColumn<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.header == other.header
            && self.sortable == other.sortable
            && self.filterable == other.filterable
            && self.hideable == other.hideable
            && self.class == other.class
            && self.head_class == other.head_class
            && self.cell_class == other.cell_class
    }
}

impl<T> DataTableColumn<T> {
    pub fn new(id: &'static str, header: &'static str, accessor: fn(&T) -> String) -> Self {
        Self {
            id,
            header,
            accessor,
            sort_value: None,
            filter: None,
            cell: None,
            header_render: None,
            cell_render: None,
            sortable: true,
            filterable: true,
            hideable: true,
            class: "",
            head_class: "",
            cell_class: "",
            dynamic_head_class: None,
            dynamic_cell_class: None,
        }
    }

    pub fn sortable(mut self, sortable: bool) -> Self {
        self.sortable = sortable;
        self
    }

    pub fn sort_value(mut self, sort_value: fn(&T) -> DataTableSortValue) -> Self {
        self.sort_value = Some(sort_value);
        self
    }

    pub fn filterable(mut self, filterable: bool) -> Self {
        self.filterable = filterable;
        self
    }

    pub fn filter(mut self, filter: fn(&T, &str) -> bool) -> Self {
        self.filter = Some(filter);
        self
    }

    pub fn cell(mut self, cell: fn(&T) -> Element) -> Self {
        self.cell = Some(cell);
        self
    }

    pub fn header_render(mut self, header_render: fn(DataTableHeaderContext) -> Element) -> Self {
        self.header_render = Some(header_render);
        self
    }

    pub fn cell_render(mut self, cell_render: fn(DataTableCellContext<T>) -> Element) -> Self {
        self.cell_render = Some(cell_render);
        self
    }

    pub fn hideable(mut self, hideable: bool) -> Self {
        self.hideable = hideable;
        self
    }

    pub fn class(mut self, class: &'static str) -> Self {
        self.class = class;
        self
    }

    pub fn head_class(mut self, class: &'static str) -> Self {
        self.head_class = class;
        self
    }

    pub fn cell_class(mut self, class: &'static str) -> Self {
        self.cell_class = class;
        self
    }

    pub fn dynamic_head_class(
        mut self,
        dynamic_head_class: fn(DataTableHeaderContext) -> String,
    ) -> Self {
        self.dynamic_head_class = Some(dynamic_head_class);
        self
    }

    pub fn dynamic_cell_class(
        mut self,
        dynamic_cell_class: fn(DataTableCellContext<T>) -> String,
    ) -> Self {
        self.dynamic_cell_class = Some(dynamic_cell_class);
        self
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DataTableHeaderContext {
    pub column_id: String,
    pub header: String,
    pub sorted: Option<DataTableSortDirection>,
}

#[derive(Clone, PartialEq)]
pub struct DataTableCellContext<T> {
    pub row_id: String,
    pub row: T,
    pub column_id: String,
    pub value: String,
    pub selected: bool,
}

#[derive(Clone, PartialEq)]
pub struct DataTableRowContext<T> {
    pub row_id: String,
    pub row: T,
    pub selected: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DataTableSortValue {
    Text(String),
    Number(i64),
    Bool(bool),
}

impl DataTableSortValue {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
            (Self::Number(a), Self::Number(b)) => a.cmp(b),
            (Self::Bool(a), Self::Bool(b)) => a.cmp(b),
            (a, b) => a.as_sort_key().cmp(&b.as_sort_key()),
        }
    }

    fn as_sort_key(&self) -> String {
        match self {
            Self::Text(value) => value.clone(),
            Self::Number(value) => value.to_string(),
            Self::Bool(value) => value.to_string(),
        }
    }
}

impl From<String> for DataTableSortValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for DataTableSortValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

impl From<i64> for DataTableSortValue {
    fn from(value: i64) -> Self {
        Self::Number(value)
    }
}

impl From<u32> for DataTableSortValue {
    fn from(value: u32) -> Self {
        Self::Number(value.into())
    }
}

impl From<bool> for DataTableSortValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DataTableSortDirection {
    Asc,
    Desc,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DataTableSort {
    pub column_id: String,
    pub direction: DataTableSortDirection,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DataTableState {
    pub sorting: Vec<DataTableSort>,
    pub global_filter: String,
    pub column_filters: HashMap<String, String>,
    pub page_index: usize,
    pub page_size: usize,
    pub column_visibility: HashMap<String, bool>,
    pub row_selection: HashSet<String>,
}

impl Default for DataTableState {
    fn default() -> Self {
        Self {
            sorting: Vec::new(),
            global_filter: String::new(),
            column_filters: HashMap::new(),
            page_index: 0,
            page_size: 10,
            column_visibility: HashMap::new(),
            row_selection: HashSet::new(),
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct DataTableModel<T> {
    pub rows: Vec<T>,
    pub columns: Vec<DataTableColumn<T>>,
    pub state: DataTableState,
    pub options: DataTableOptions<T>,
}

#[derive(Clone)]
pub struct DataTableOptions<T> {
    pub get_row_id: fn(&T, usize) -> String,
    pub row_class: Option<fn(DataTableRowContext<T>) -> String>,
    pub manual_filtering: bool,
    pub manual_sorting: bool,
    pub manual_pagination: bool,
    pub total_rows: Option<usize>,
}

impl<T> PartialEq for DataTableOptions<T> {
    fn eq(&self, other: &Self) -> bool {
        self.manual_filtering == other.manual_filtering
            && self.manual_sorting == other.manual_sorting
            && self.manual_pagination == other.manual_pagination
            && self.total_rows == other.total_rows
    }
}

impl<T> Default for DataTableOptions<T> {
    fn default() -> Self {
        Self {
            get_row_id: default_row_id::<T>,
            row_class: None,
            manual_filtering: false,
            manual_sorting: false,
            manual_pagination: false,
            total_rows: None,
        }
    }
}

fn default_row_id<T>(_row: &T, index: usize) -> String {
    index.to_string()
}

impl<T> DataTableOptions<T> {
    pub fn get_row_id(mut self, get_row_id: fn(&T, usize) -> String) -> Self {
        self.get_row_id = get_row_id;
        self
    }

    pub fn row_class(mut self, row_class: fn(DataTableRowContext<T>) -> String) -> Self {
        self.row_class = Some(row_class);
        self
    }

    pub fn manual_filtering(mut self, manual_filtering: bool) -> Self {
        self.manual_filtering = manual_filtering;
        self
    }

    pub fn manual_sorting(mut self, manual_sorting: bool) -> Self {
        self.manual_sorting = manual_sorting;
        self
    }

    pub fn manual_pagination(mut self, manual_pagination: bool) -> Self {
        self.manual_pagination = manual_pagination;
        self
    }

    pub fn total_rows(mut self, total_rows: usize) -> Self {
        self.total_rows = Some(total_rows);
        self
    }
}

#[derive(Clone, PartialEq)]
pub struct DataTableRow<T> {
    pub id: String,
    pub original: T,
    pub selected: bool,
}

impl<T: Clone> DataTableModel<T> {
    pub fn new(rows: Vec<T>, columns: Vec<DataTableColumn<T>>, state: DataTableState) -> Self {
        Self::with_options(rows, columns, state, DataTableOptions::default())
    }

    pub fn with_options(
        rows: Vec<T>,
        columns: Vec<DataTableColumn<T>>,
        state: DataTableState,
        options: DataTableOptions<T>,
    ) -> Self {
        Self {
            rows,
            columns,
            state,
            options,
        }
    }

    pub fn visible_columns(&self) -> Vec<&DataTableColumn<T>> {
        self.columns
            .iter()
            .filter(|column| {
                self.state
                    .column_visibility
                    .get(column.id)
                    .copied()
                    .unwrap_or(true)
            })
            .collect()
    }

    pub fn row_model(&self) -> Vec<T> {
        self.resolved_rows()
            .into_iter()
            .map(|row| row.original)
            .collect()
    }

    pub fn resolved_rows(&self) -> Vec<DataTableRow<T>> {
        let global_filter = self.state.global_filter.trim().to_lowercase();
        let mut rows = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                if self.options.manual_filtering {
                    return true;
                }

                let global_matches = if global_filter.is_empty() {
                    true
                } else {
                    self.columns
                        .iter()
                        .filter(|column| column.filterable)
                        .any(|column| column_matches_filter(column, row, &global_filter))
                };

                let column_matches = self
                    .state
                    .column_filters
                    .iter()
                    .filter(|(_, filter)| !filter.trim().is_empty())
                    .all(|(column_id, filter)| {
                        self.columns
                            .iter()
                            .find(|column| column.id == column_id && column.filterable)
                            .map(|column| {
                                column_matches_filter(column, row, &filter.to_lowercase())
                            })
                            .unwrap_or(true)
                    });

                global_matches && column_matches
            })
            .map(|(index, row)| {
                let id = (self.options.get_row_id)(row, index);
                DataTableRow {
                    selected: self.state.row_selection.contains(&id),
                    id,
                    original: row.clone(),
                }
            })
            .collect::<Vec<_>>();

        if !self.options.manual_sorting {
            for sort in self.state.sorting.iter().rev() {
                if let Some(column) = self
                    .columns
                    .iter()
                    .find(|column| column.id == sort.column_id && column.sortable)
                {
                    rows.sort_by(|a, b| {
                        let ordering =
                            sort_value(column, &a.original).cmp(&sort_value(column, &b.original));
                        match sort.direction {
                            DataTableSortDirection::Asc => ordering,
                            DataTableSortDirection::Desc => ordering.reverse(),
                        }
                    });
                }
            }
        }

        rows
    }

    pub fn page_count(&self) -> usize {
        let page_size = self.state.page_size.max(1);
        let total = self
            .options
            .total_rows
            .unwrap_or_else(|| self.resolved_rows().len());
        total.div_ceil(page_size)
    }

    pub fn page_rows(&self) -> Vec<T> {
        self.page_row_model()
            .into_iter()
            .map(|row| row.original)
            .collect()
    }

    pub fn page_row_model(&self) -> Vec<DataTableRow<T>> {
        let rows = self.resolved_rows();
        if self.options.manual_pagination {
            return rows;
        }

        let page_size = self.state.page_size.max(1);
        let start = self.state.page_index.saturating_mul(page_size);
        rows.into_iter().skip(start).take(page_size).collect()
    }

    pub fn selected_rows(&self) -> Vec<DataTableRow<T>> {
        self.resolved_rows()
            .into_iter()
            .filter(|row| row.selected)
            .collect()
    }

    pub fn selected_count(&self) -> usize {
        self.state.row_selection.len()
    }
}

fn column_matches_filter<T>(column: &DataTableColumn<T>, row: &T, filter: &str) -> bool {
    if let Some(filter_fn) = column.filter {
        filter_fn(row, filter)
    } else {
        (column.accessor)(row).to_lowercase().contains(filter)
    }
}

fn sort_value<T>(column: &DataTableColumn<T>, row: &T) -> DataTableSortValue {
    if let Some(sort_value) = column.sort_value {
        sort_value(row)
    } else {
        DataTableSortValue::Text((column.accessor)(row))
    }
}

#[derive(Clone, Copy, PartialEq)]
pub struct DataTableApi<T: Clone + PartialEq + 'static> {
    rows: ReadSignal<Vec<T>>,
    columns: ReadSignal<Vec<DataTableColumn<T>>>,
    options: ReadSignal<DataTableOptions<T>>,
    state: Signal<DataTableState>,
    on_state_change: Option<Callback<DataTableState>>,
}

pub fn use_data_table<T: Clone + PartialEq + 'static>(
    rows: impl Into<ReadSignal<Vec<T>>>,
    columns: impl Into<ReadSignal<Vec<DataTableColumn<T>>>>,
) -> DataTableApi<T> {
    let options = use_signal(DataTableOptions::default);
    use_data_table_with_options(rows, columns, options)
}

pub fn use_data_table_with_options<T: Clone + PartialEq + 'static>(
    rows: impl Into<ReadSignal<Vec<T>>>,
    columns: impl Into<ReadSignal<Vec<DataTableColumn<T>>>>,
    options: impl Into<ReadSignal<DataTableOptions<T>>>,
) -> DataTableApi<T> {
    DataTableApi {
        rows: rows.into(),
        columns: columns.into(),
        options: options.into(),
        state: use_signal(DataTableState::default),
        on_state_change: None,
    }
}

impl<T: Clone + PartialEq + 'static> DataTableApi<T> {
    pub fn state(&self) -> DataTableState {
        (self.state)()
    }

    pub fn set_state(&mut self, state: DataTableState) {
        self.state.set(state.clone());
        if let Some(callback) = &self.on_state_change {
            callback.call(state);
        }
    }

    pub fn model(&self) -> DataTableModel<T> {
        DataTableModel::with_options(
            (self.rows)(),
            (self.columns)(),
            self.state(),
            (self.options)(),
        )
    }

    pub fn with_on_state_change(mut self, on_state_change: Callback<DataTableState>) -> Self {
        self.on_state_change = Some(on_state_change);
        self
    }

    fn update_state(&mut self, update: impl FnOnce(&mut DataTableState)) {
        let mut next = self.state();
        update(&mut next);
        self.set_state(next);
    }

    pub fn set_global_filter(&mut self, filter: impl Into<String>) {
        self.update_state(|state| {
            state.global_filter = filter.into();
            state.page_index = 0;
        });
    }

    pub fn set_column_filter(&mut self, column_id: impl Into<String>, filter: impl Into<String>) {
        let column_id = column_id.into();
        let filter = filter.into();
        self.update_state(|state| {
            if filter.trim().is_empty() {
                state.column_filters.remove(&column_id);
            } else {
                state.column_filters.insert(column_id, filter);
            }
            state.page_index = 0;
        });
    }

    pub fn set_page_size(&mut self, page_size: usize) {
        self.update_state(|state| {
            state.page_size = page_size.max(1);
            state.page_index = 0;
        });
    }

    pub fn next_page(&mut self) {
        let page_count = self.model().page_count();
        self.update_state(|state| {
            if state.page_index + 1 < page_count {
                state.page_index += 1;
            }
        });
    }

    pub fn previous_page(&mut self) {
        self.update_state(|state| {
            state.page_index = state.page_index.saturating_sub(1);
        });
    }

    pub fn set_page_index(&mut self, page_index: usize) {
        let max_page_index = self.model().page_count().saturating_sub(1);
        self.update_state(|state| {
            state.page_index = page_index.min(max_page_index);
        });
    }

    pub fn toggle_sort(&mut self, column_id: impl Into<String>) {
        let column_id = column_id.into();
        self.update_state(|state| {
            if let Some(sort) = state
                .sorting
                .iter_mut()
                .find(|sort| sort.column_id == column_id)
            {
                match sort.direction {
                    DataTableSortDirection::Asc => {
                        sort.direction = DataTableSortDirection::Desc;
                    }
                    DataTableSortDirection::Desc => {
                        state.sorting.retain(|sort| sort.column_id != column_id);
                    }
                }
            } else {
                state.sorting.push(DataTableSort {
                    column_id,
                    direction: DataTableSortDirection::Asc,
                });
            }
        });
    }

    pub fn set_column_visible(&mut self, column_id: impl Into<String>, visible: bool) {
        let column_id = column_id.into();
        self.update_state(|state| {
            state.column_visibility.insert(column_id, visible);
        });
    }

    pub fn toggle_row_selected(&mut self, row_id: impl Into<String>) {
        let row_id = row_id.into();
        self.update_state(|state| {
            if !state.row_selection.remove(&row_id) {
                state.row_selection.insert(row_id);
            }
        });
    }

    pub fn set_row_selected(&mut self, row_id: impl Into<String>, selected: bool) {
        let row_id = row_id.into();
        self.update_state(|state| {
            if selected {
                state.row_selection.insert(row_id);
            } else {
                state.row_selection.remove(&row_id);
            }
        });
    }

    pub fn toggle_all_page_rows_selected(&mut self) {
        let rows = self.model().page_row_model();
        let all_selected = !rows.is_empty() && rows.iter().all(|row| row.selected);
        self.update_state(|state| {
            for row in rows {
                if all_selected {
                    state.row_selection.remove(&row.id);
                } else {
                    state.row_selection.insert(row.id);
                }
            }
        });
    }

    pub fn all_page_rows_selected(&self) -> bool {
        let rows = self.model().page_row_model();
        !rows.is_empty() && rows.iter().all(|row| row.selected)
    }

    pub fn some_page_rows_selected(&self) -> bool {
        let rows = self.model().page_row_model();
        rows.iter().any(|row| row.selected) && !rows.iter().all(|row| row.selected)
    }

    pub fn selected_count(&self) -> usize {
        self.state().row_selection.len()
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DataTableFilterProps<T: Clone + PartialEq + 'static> {
    pub table: DataTableApi<T>,
    #[props(default = "Search...".to_string())]
    pub placeholder: String,
    #[props(default)]
    pub class: String,
}

#[component]
pub fn DataTableFilter<T: Clone + PartialEq + 'static>(
    mut props: DataTableFilterProps<T>,
) -> Element {
    rsx! {
        input {
            class: crate::cn::merge_slice(&[
                "h-9 w-full max-w-sm rounded-lg border border-input bg-transparent px-3 py-1 text-sm shadow-xs transition-colors placeholder:text-muted-foreground focus-visible:border-ring focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50",
                props.class.as_str(),
            ]),
            value: "{props.table.state().global_filter}",
            placeholder: "{props.placeholder}",
            oninput: move |event| props.table.set_global_filter(event.value()),
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DataTablePaginationProps {
    pub page_index: usize,
    pub page_count: usize,
    pub on_previous: Callback<MouseEvent>,
    pub on_next: Callback<MouseEvent>,
    #[props(default)]
    pub class: String,
}

#[component]
pub fn DataTablePagination(props: DataTablePaginationProps) -> Element {
    let can_previous = props.page_index > 0;
    let can_next = props.page_index + 1 < props.page_count;

    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex items-center justify-end gap-2", props.class.as_str()]),
            span {
                class: "text-sm text-muted-foreground",
                "Page {props.page_index + 1} of {props.page_count.max(1)}"
            }
            button {
                class: "inline-flex h-8 items-center justify-center rounded-lg border border-input bg-background px-3 text-sm shadow-xs transition-colors hover:bg-accent hover:text-accent-foreground disabled:pointer-events-none disabled:opacity-50",
                disabled: !can_previous,
                onclick: move |event| props.on_previous.call(event),
                "Previous"
            }
            button {
                class: "inline-flex h-8 items-center justify-center rounded-lg border border-input bg-background px-3 text-sm shadow-xs transition-colors hover:bg-accent hover:text-accent-foreground disabled:pointer-events-none disabled:opacity-50",
                disabled: !can_next,
                onclick: move |event| props.on_next.call(event),
                "Next"
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DataTableViewProps<T: Clone + PartialEq + 'static> {
    pub table: DataTableApi<T>,
    #[props(default = false)]
    pub selectable: bool,
    #[props(default = "No results.".to_string())]
    pub empty: String,
    #[props(default)]
    pub class: String,
}

#[component]
pub fn DataTableView<T: Clone + PartialEq + 'static>(props: DataTableViewProps<T>) -> Element {
    let model = props.table.model();
    let columns = model.visible_columns();
    let rows = model.page_row_model();
    let total_columns = columns.len() + usize::from(props.selectable);

    rsx! {
        div {
            class: crate::cn::merge_slice(&["rounded-lg border border-border", props.class.as_str()]),
            TableContainer {
                Table {
                    TableHeader {
                        TableRow {
                            if props.selectable {
                                TableHead {
                                    class: "w-10",
                                    input {
                                        class: "size-4 rounded border border-input accent-primary",
                                        r#type: "checkbox",
                                        checked: props.table.all_page_rows_selected(),
                                        "aria-label": "Select all rows",
                                        onchange: {
                                            let mut table = props.table.clone();
                                            move |_| table.toggle_all_page_rows_selected()
                                        },
                                    }
                                }
                            }
                            for column in columns.iter().cloned().cloned() {
                                DataTableHeaderCell {
                                    key: "{column.id}",
                                    table: props.table.clone(),
                                    state: model.state.clone(),
                                    column,
                                }
                            }
                        }
                    }
                    TableBody {
                        if rows.is_empty() {
                            TableRow {
                                td {
                                    class: "h-24 p-3 text-center align-middle text-muted-foreground",
                                    colspan: total_columns.max(1).to_string(),
                                    "{props.empty}"
                                }
                            }
                        } else {
                            for row in rows.iter().cloned() {
                                DataTableBodyRow {
                                    key: "{row.id}",
                                    table: props.table.clone(),
                                    columns: columns.iter().cloned().cloned().collect::<Vec<_>>(),
                                    row,
                                    row_class: model.options.row_class,
                                    selectable: props.selectable,
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct DataTableHeaderCellProps<T: Clone + PartialEq + 'static> {
    table: DataTableApi<T>,
    state: DataTableState,
    column: DataTableColumn<T>,
}

#[component]
fn DataTableHeaderCell<T: Clone + PartialEq + 'static>(
    props: DataTableHeaderCellProps<T>,
) -> Element {
    let context = header_context(&props.state, &props.column);
    let dynamic_class = props
        .column
        .dynamic_head_class
        .map(|class_fn| class_fn(context.clone()))
        .unwrap_or_default();

    rsx! {
        TableHead {
            class: crate::cn::merge_slice(&[
                props.column.class,
                props.column.head_class,
                dynamic_class.as_str(),
            ]),
            if props.column.sortable {
                button {
                    class: "inline-flex items-center gap-1 rounded-md transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50",
                    onclick: {
                        let column_id = props.column.id;
                        let mut table = props.table.clone();
                        move |_| table.toggle_sort(column_id)
                    },
                    if let Some(header_render) = props.column.header_render {
                        {header_render(context.clone())}
                    } else {
                        "{props.column.header}"
                    }
                    span {
                        class: "text-muted-foreground text-xs",
                        "{sort_indicator(&props.state, props.column.id)}"
                    }
                }
            } else if let Some(header_render) = props.column.header_render {
                {header_render(context)}
            } else {
                "{props.column.header}"
            }
        }
    }
}

#[derive(Props, Clone)]
struct DataTableBodyRowProps<T: Clone + PartialEq + 'static> {
    table: DataTableApi<T>,
    columns: Vec<DataTableColumn<T>>,
    row: DataTableRow<T>,
    row_class: Option<fn(DataTableRowContext<T>) -> String>,
    selectable: bool,
}

impl<T: Clone + PartialEq + 'static> PartialEq for DataTableBodyRowProps<T> {
    fn eq(&self, other: &Self) -> bool {
        self.table == other.table
            && self.columns == other.columns
            && self.row == other.row
            && self.selectable == other.selectable
    }
}

#[component]
fn DataTableBodyRow<T: Clone + PartialEq + 'static>(props: DataTableBodyRowProps<T>) -> Element {
    let row_class = props
        .row_class
        .map(|class_fn| class_fn(row_context(&props.row)))
        .unwrap_or_default();

    rsx! {
        TableRow {
            class: row_class,
            if props.selectable {
                TableCell {
                    class: "w-10",
                    input {
                        class: "size-4 rounded border border-input accent-primary",
                        r#type: "checkbox",
                        checked: props.row.selected,
                        "aria-label": "Select row",
                        onchange: {
                            let row_id = props.row.id.clone();
                            let mut table = props.table.clone();
                            move |_| table.toggle_row_selected(row_id.clone())
                        },
                    }
                }
            }
            for column in props.columns.iter().cloned() {
                DataTableBodyCell {
                    key: "{column.id}",
                    row: props.row.clone(),
                    column,
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct DataTableBodyCellProps<T: Clone + PartialEq + 'static> {
    row: DataTableRow<T>,
    column: DataTableColumn<T>,
}

#[component]
fn DataTableBodyCell<T: Clone + PartialEq + 'static>(props: DataTableBodyCellProps<T>) -> Element {
    let context = cell_context(&props.row, &props.column);
    let dynamic_class = props
        .column
        .dynamic_cell_class
        .map(|class_fn| class_fn(context.clone()))
        .unwrap_or_default();

    rsx! {
        TableCell {
            class: crate::cn::merge_slice(&[
                props.column.class,
                props.column.cell_class,
                dynamic_class.as_str(),
            ]),
            if let Some(cell_render) = props.column.cell_render {
                {cell_render(context.clone())}
            } else if let Some(cell) = props.column.cell {
                {cell(&props.row.original)}
            } else {
                "{(props.column.accessor)(&props.row.original)}"
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DataTableToolbarProps<T: Clone + PartialEq + 'static> {
    pub table: DataTableApi<T>,
    #[props(default = "Search...".to_string())]
    pub search_placeholder: String,
    #[props(default = vec![10, 20, 50, 100])]
    pub page_sizes: Vec<usize>,
    #[props(default = true)]
    pub show_search: bool,
    #[props(default = true)]
    pub show_column_visibility: bool,
    #[props(default = true)]
    pub show_selected_count: bool,
    #[props(default = true)]
    pub show_page_size: bool,
    #[props(default)]
    pub class: String,
}

#[component]
pub fn DataTableToolbar<T: Clone + PartialEq + 'static>(
    props: DataTableToolbarProps<T>,
) -> Element {
    let model = props.table.model();
    let selected_count = props.table.selected_count();
    let current_page_size = model.state.page_size.to_string();
    let mut page_size_value = use_signal(|| current_page_size.clone());

    use_effect(move || {
        page_size_value.set(current_page_size.clone());
    });

    rsx! {
        div {
            class: crate::cn::merge_slice(&[
                "flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between",
                props.class.as_str(),
            ]),
            div {
                class: "flex min-w-0 flex-1 items-center gap-2",
                if props.show_search {
                    DataTableFilter {
                        table: props.table.clone(),
                        placeholder: props.search_placeholder.clone(),
                    }
                }
                if props.show_selected_count && selected_count > 0 {
                    span {
                        class: "whitespace-nowrap text-sm text-muted-foreground",
                        "{selected_count} selected"
                    }
                }
            }
            div {
                class: "flex items-center gap-2",
                if props.show_column_visibility {
                    DataTableColumnVisibility {
                        table: props.table.clone(),
                    }
                }
                if props.show_page_size {
                    Select {
                        value: page_size_value,
                        class: "w-32".to_string(),
                        on_change: {
                            let mut table = props.table.clone();
                            move |value: String| {
                                if let Ok(page_size) = value.parse::<usize>() {
                                    table.set_page_size(page_size);
                                }
                            }
                        },
                        SelectContent {
                            for (index, page_size) in props.page_sizes.iter().copied().enumerate() {
                                SelectItem {
                                    key: "{page_size}",
                                    value: page_size.to_string(),
                                    index,
                                    "{page_size} rows"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DataTableColumnVisibilityProps<T: Clone + PartialEq + 'static> {
    pub table: DataTableApi<T>,
    #[props(default = "Columns".to_string())]
    pub label: String,
    #[props(default)]
    pub class: String,
}

#[component]
pub fn DataTableColumnVisibility<T: Clone + PartialEq + 'static>(
    props: DataTableColumnVisibilityProps<T>,
) -> Element {
    let model = props.table.model();

    rsx! {
        details {
            class: crate::cn::merge_slice(&["relative", props.class.as_str()]),
            summary {
                class: "inline-flex h-9 cursor-pointer list-none items-center justify-center rounded-lg border border-input bg-background px-3 text-sm shadow-xs transition-colors hover:bg-accent hover:text-accent-foreground [&::-webkit-details-marker]:hidden",
                "{props.label}"
            }
            div {
                class: "absolute right-0 z-50 mt-2 grid min-w-44 gap-1 rounded-lg border border-border bg-popover p-1 text-popover-foreground shadow-md",
                for column in model.columns.iter().filter(|column| column.hideable) {
                    label {
                        key: "{column.id}",
                        class: "flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-sm hover:bg-accent hover:text-accent-foreground",
                        input {
                            class: "size-4 rounded border border-input accent-primary",
                            r#type: "checkbox",
                            checked: model
                                .state
                                .column_visibility
                                .get(column.id)
                                .copied()
                                .unwrap_or(true),
                            onchange: {
                                let column_id = column.id;
                                let mut table = props.table.clone();
                                move |event| table.set_column_visible(column_id, event.checked())
                            },
                        }
                        "{column.header}"
                    }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DataTableFooterProps<T: Clone + PartialEq + 'static> {
    pub table: DataTableApi<T>,
    #[props(default)]
    pub class: String,
}

#[component]
pub fn DataTableFooter<T: Clone + PartialEq + 'static>(props: DataTableFooterProps<T>) -> Element {
    let model = props.table.model();

    rsx! {
        DataTablePagination {
            page_index: model.state.page_index,
            page_count: model.page_count(),
            on_previous: {
                let mut table = props.table.clone();
                move |_| table.previous_page()
            },
            on_next: {
                let mut table = props.table.clone();
                move |_| table.next_page()
            },
            class: props.class,
        }
    }
}

fn header_context<T>(
    state: &DataTableState,
    column: &DataTableColumn<T>,
) -> DataTableHeaderContext {
    DataTableHeaderContext {
        column_id: column.id.to_string(),
        header: column.header.to_string(),
        sorted: state
            .sorting
            .iter()
            .find(|sort| sort.column_id == column.id)
            .map(|sort| sort.direction),
    }
}

fn cell_context<T: Clone>(
    row: &DataTableRow<T>,
    column: &DataTableColumn<T>,
) -> DataTableCellContext<T> {
    DataTableCellContext {
        row_id: row.id.clone(),
        row: row.original.clone(),
        column_id: column.id.to_string(),
        value: (column.accessor)(&row.original),
        selected: row.selected,
    }
}

fn row_context<T: Clone>(row: &DataTableRow<T>) -> DataTableRowContext<T> {
    DataTableRowContext {
        row_id: row.id.clone(),
        row: row.original.clone(),
        selected: row.selected,
    }
}

fn sort_indicator(state: &DataTableState, column_id: &str) -> &'static str {
    state
        .sorting
        .iter()
        .find(|sort| sort.column_id == column_id)
        .map(|sort| match sort.direction {
            DataTableSortDirection::Asc => "Asc",
            DataTableSortDirection::Desc => "Desc",
        })
        .unwrap_or("Sort")
}

#[derive(Clone, PartialEq)]
struct DataTableStoryTask {
    id: u32,
    title: &'static str,
    status: &'static str,
    owner: &'static str,
    priority: u32,
}

fn data_table_story_tasks() -> Vec<DataTableStoryTask> {
    vec![
        DataTableStoryTask {
            id: 101,
            title: "Wire account settings",
            status: "In Progress",
            owner: "Ada",
            priority: 3,
        },
        DataTableStoryTask {
            id: 102,
            title: "Review invoice export",
            status: "Queued",
            owner: "Grace",
            priority: 2,
        },
        DataTableStoryTask {
            id: 103,
            title: "Ship mobile shell",
            status: "Blocked",
            owner: "Linus",
            priority: 4,
        },
        DataTableStoryTask {
            id: 104,
            title: "Tighten data table API",
            status: "Done",
            owner: "Barbara",
            priority: 1,
        },
    ]
}

fn data_table_story_columns() -> Vec<DataTableColumn<DataTableStoryTask>> {
    vec![
        DataTableColumn::new("title", "Task", |task: &DataTableStoryTask| {
            task.title.to_string()
        }),
        DataTableColumn::new("status", "Status", |task: &DataTableStoryTask| {
            task.status.to_string()
        }),
        DataTableColumn::new("owner", "Owner", |task: &DataTableStoryTask| {
            task.owner.to_string()
        }),
        DataTableColumn::new("priority", "Priority", |task: &DataTableStoryTask| {
            task.priority.to_string()
        })
        .sort_value(|task| task.priority.into())
        .cell_class("text-right")
        .head_class("text-right"),
    ]
}

/// Default DataTable story with toolbar, view, and footer wiring.
#[story(category = "DataTable", name = "default")]
pub fn data_table_default() -> Element {
    let rows = use_signal(data_table_story_tasks);
    let columns = use_signal(data_table_story_columns);
    let options = use_signal(|| {
        DataTableOptions::default().get_row_id(|task: &DataTableStoryTask, _| task.id.to_string())
    });
    let table = use_data_table_with_options(rows, columns, options);

    rsx! {
        div { class: "p-6 bg-background text-foreground",
            div { class: "flex flex-col gap-3",
                DataTableToolbar { table: table.clone() }
                DataTableView {
                    table: table.clone(),
                    selectable: true,
                    empty: "No tasks match the current filters.".to_string(),
                }
                DataTableFooter { table }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, PartialEq)]
    struct User {
        id: u32,
        name: &'static str,
        role: &'static str,
    }

    fn users() -> Vec<User> {
        vec![
            User {
                id: 1,
                name: "Ada",
                role: "Admin",
            },
            User {
                id: 2,
                name: "Grace",
                role: "Editor",
            },
        ]
    }

    #[test]
    fn filters_rows() {
        let mut state = DataTableState::default();
        state.global_filter = "edit".into();
        let table = DataTableModel::new(
            users(),
            vec![DataTableColumn::new("role", "Role", |user: &User| {
                user.role.to_string()
            })],
            state,
        );

        assert_eq!(table.row_model().len(), 1);
        assert_eq!(table.row_model()[0].name, "Grace");
    }

    #[test]
    fn sorts_rows() {
        let mut state = DataTableState::default();
        state.sorting = vec![DataTableSort {
            column_id: "name".into(),
            direction: DataTableSortDirection::Desc,
        }];
        let table = DataTableModel::new(
            users(),
            vec![DataTableColumn::new("name", "Name", |user: &User| {
                user.name.to_string()
            })],
            state,
        );

        assert_eq!(table.row_model()[0].name, "Grace");
    }

    #[test]
    fn paginates_rows() {
        let state = DataTableState {
            page_index: 1,
            page_size: 1,
            ..Default::default()
        };
        let table = DataTableModel::new(
            users(),
            vec![DataTableColumn::new("id", "ID", |user: &User| {
                user.id.to_string()
            })],
            state,
        );

        assert_eq!(table.page_count(), 2);
        assert_eq!(table.page_rows()[0].name, "Grace");
    }

    #[test]
    fn filters_by_column() {
        let mut state = DataTableState::default();
        state.column_filters.insert("name".into(), "ada".into());
        let table = DataTableModel::new(
            users(),
            vec![
                DataTableColumn::new("name", "Name", |user: &User| user.name.to_string()),
                DataTableColumn::new("role", "Role", |user: &User| user.role.to_string()),
            ],
            state,
        );

        assert_eq!(table.row_model().len(), 1);
        assert_eq!(table.row_model()[0].name, "Ada");
    }

    #[test]
    fn uses_typed_sort_values() {
        let mut state = DataTableState::default();
        state.sorting = vec![DataTableSort {
            column_id: "id".into(),
            direction: DataTableSortDirection::Desc,
        }];
        let table = DataTableModel::new(
            users(),
            vec![
                DataTableColumn::new("id", "ID", |user: &User| user.id.to_string())
                    .sort_value(|user| user.id.into()),
            ],
            state,
        );

        assert_eq!(table.row_model()[0].id, 2);
    }

    #[test]
    fn tracks_selection_with_stable_row_ids() {
        let state = DataTableState {
            row_selection: HashSet::from(["user-2".to_string()]),
            ..Default::default()
        };
        let table = DataTableModel::with_options(
            users(),
            vec![DataTableColumn::new("name", "Name", |user: &User| {
                user.name.to_string()
            })],
            state,
            DataTableOptions::default().get_row_id(|user, _| format!("user-{}", user.id)),
        );

        assert_eq!(table.selected_rows().len(), 1);
        assert_eq!(table.selected_rows()[0].original.name, "Grace");
    }

    #[test]
    fn manual_pagination_uses_supplied_rows_and_total() {
        let state = DataTableState {
            page_index: 1,
            page_size: 2,
            ..Default::default()
        };
        let table = DataTableModel::with_options(
            users(),
            vec![DataTableColumn::new("name", "Name", |user: &User| {
                user.name.to_string()
            })],
            state,
            DataTableOptions::default()
                .manual_pagination(true)
                .total_rows(10),
        );

        assert_eq!(table.page_count(), 5);
        assert_eq!(table.page_row_model().len(), 2);
    }

    #[test]
    fn builds_header_cell_and_row_contexts() {
        let state = DataTableState {
            sorting: vec![DataTableSort {
                column_id: "name".into(),
                direction: DataTableSortDirection::Asc,
            }],
            row_selection: HashSet::from(["user-1".to_string()]),
            ..Default::default()
        };
        let column = DataTableColumn::new("name", "Name", |user: &User| user.name.to_string())
            .dynamic_head_class(|context| {
                if context.sorted.is_some() {
                    "text-primary".to_string()
                } else {
                    String::new()
                }
            })
            .dynamic_cell_class(|context| {
                if context.selected {
                    "font-medium".to_string()
                } else {
                    String::new()
                }
            });
        let table = DataTableModel::with_options(
            users(),
            vec![column.clone()],
            state,
            DataTableOptions::default().get_row_id(|user, _| format!("user-{}", user.id)),
        );
        let row = table.page_row_model()[0].clone();
        let header = header_context(&table.state, &column);
        let cell = cell_context(&row, &column);
        let row_ctx = row_context(&row);

        assert_eq!(header.sorted, Some(DataTableSortDirection::Asc));
        assert_eq!(cell.value, "Ada");
        assert!(cell.selected);
        assert_eq!(row_ctx.row_id, "user-1");
        assert_eq!(
            column.dynamic_head_class.unwrap()(header),
            "text-primary".to_string()
        );
        assert_eq!(
            column.dynamic_cell_class.unwrap()(cell),
            "font-medium".to_string()
        );
    }
}
