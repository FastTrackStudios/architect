//! Convenience re-exports — `use architect_ui::prelude::*`.

// Components — core
pub use crate::components::{
    use_data_table, use_data_table_with_options, use_direction, use_form, Accordion,
    AccordionContent, AccordionItem, AccordionTrigger, Badge, BadgeVariant, Button, ButtonGroup,
    ButtonGroupOrientation, ButtonGroupText, ButtonRenderAs, ButtonSize, ButtonVariant, Card,
    CardContent, CardDescription, CardFooter, CardHeader, CardTitle, Checkbox, Collapsible,
    CollapsibleContent, CollapsibleTrigger, ContextMenu, ContextMenuContent, ContextMenuItem,
    ContextMenuLabel, ContextMenuSeparator, ContextMenuTrigger, DataTableApi, DataTableCellContext,
    DataTableColumn, DataTableColumnVisibility, DataTableFilter, DataTableFooter,
    DataTableHeaderContext, DataTableModel, DataTableOptions, DataTablePagination, DataTableRow,
    DataTableRowContext, DataTableSort, DataTableSortDirection, DataTableSortValue, DataTableState,
    DataTableToolbar, DataTableView, Dialog, DialogClose, DialogDescription, DialogFooter,
    DialogHeader, DialogTitle, Direction, DirectionContext, DirectionProvider, Dropdown,
    DropdownContent, DropdownItem, DropdownLabel, DropdownSeparator, DropdownTrigger, EmptyState,
    FieldApi, Form, FormApi, FormField, FormFieldState, FormMessage, FormRule, FormState,
    HoverCard, HoverCardContent, HoverCardTrigger, InlineEdit, Input, InputSize, InputVariant,
    Item, ItemActions, ItemContent, ItemDescription, ItemGroup, ItemMedia, ItemSize, ItemTitle,
    ItemVariant, KeyValueRow, Label, ListRow, NativeSelect, NativeSelectOption, NavTab, Progress,
    ProgressBar, ProgressBarOrientation, ProgressVariant, SearchableDropdown, SearchableList,
    SearchableListItem, SectionHeader, SectionHeaderSize, SegmentedControl, SegmentedControlSize,
    Sheet, SheetDescription, SheetFooter, SheetHeader, SheetSide, SheetTitle, StatusBadge,
    StatusBadgeVariant, StatusDot, StatusDotColor, StatusDotSize, Switch, TabContent, TabList,
    TabTrigger, Tabs,
};

// Components — shadcn v4 parity
pub use crate::components::{
    use_sidebar_compact, use_toast, Alert, AlertDescription, AlertDialog, AlertDialogAction,
    AlertDialogCancel, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader,
    AlertDialogTitle, AlertTitle, AlertVariant, AspectRatio, Avatar, AvatarFallback, AvatarImage,
    AvatarSize, Breadcrumb, BreadcrumbItem, BreadcrumbLink, BreadcrumbList, BreadcrumbPage,
    BreadcrumbSeparator, Calendar, Carousel, CarouselContent, CarouselItem, CarouselNext,
    CarouselPrevious, Combobox, ComboboxEmpty, ComboboxInput, ComboboxList, ComboboxOption,
    CommandDialog, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList,
    CommandSeparator, CommandShortcut, DatePicker, DatePickerCalendar, DatePickerContent,
    DatePickerInput, DatePickerPopover, DatePickerTrigger, Drawer, DrawerDescription, DrawerFooter,
    DrawerHandle, DrawerHeader, DrawerSide, DrawerTitle, Field, FieldDescription, FieldLabel,
    FieldMessage, InputGroup, InputGroupAddon, InputGroupAlign, InputGroupControl, InputOtp,
    InputOtpGroup, InputOtpSeparator, InputOtpSlot, Kbd, Menubar, MenubarContent, MenubarItem,
    MenubarLabel, MenubarMenu, MenubarSeparator, MenubarShortcut, MenubarTrigger, NavigationMenu,
    NavigationMenuContent, NavigationMenuItem, NavigationMenuLink, NavigationMenuList,
    NavigationMenuTrigger, Pagination, PaginationContent, PaginationEllipsis, PaginationItem,
    PaginationLink, PaginationNext, PaginationPrevious, Popover, PopoverContent, PopoverTrigger,
    RadioGroup, RadioGroupItem, ResizableContext, ResizableHandle, ResizablePanel,
    ResizablePanelGroup, ResizeDirection, ScrollArea, Select, SelectContent, SelectGroup,
    SelectItem, SelectLabel, SelectSeparator, SelectTrigger, Sidebar, SidebarCollapsible,
    SidebarContent, SidebarContext, SidebarFooter, SidebarGroup, SidebarGroupContent,
    SidebarGroupLabel, SidebarHeader, SidebarInset, SidebarLabel, SidebarMenu, SidebarMenuBadge,
    SidebarMenuButton, SidebarMenuButtonSize, SidebarMenuButtonVariant, SidebarMenuItem,
    SidebarProvider, SidebarSeparator, SidebarSide, SidebarTrigger, SidebarVariant, Skeleton,
    SkeletonCircle, SkeletonText, Slider, Spinner, SpinnerSize, Table, TableBody, TableCaption,
    TableCell, TableContainer, TableFooter, TableHead, TableHeader, TableRow, Textarea, Toast,
    ToastOptions, ToastProvider, ToastType, Toggle, ToggleGroup, ToggleGroupItem, ToggleSize,
    ToggleVariant, Toolbar, ToolbarButton, ToolbarSeparator, Tooltip, TooltipContent, TooltipSide,
    TooltipTrigger,
};

#[cfg(feature = "router")]
pub use crate::components::{Navbar, NavbarContent, NavbarItem, NavbarNav, NavbarTrigger};

// Toast (used via toast::use_toast(), toast::ToastProvider, etc.)
pub use crate::components::toast;

// Layout
pub use crate::layout::{Divider, DividerOrientation, HStack, Spacer, VStack};

// Typography
pub use crate::typography::{Heading, HeadingLevel, Text, TextVariant};

// Theme runtime and tokens
pub use crate::theme::{
    default_theme_preset, default_theme_state, theme_preset, theme_presets, tokens, use_theme,
    ThemeContext, ThemeMode, ThemePreset, ThemeProvider, ThemeRuntimeStyle, ThemeScope, ThemeState,
    ThemeStyle, ThemeStyles, ThemeSwitcher, ThemeToken,
};

// Class merging utilities. The primary component API is the `architect_ui::cn!`
// macro; these helpers cover already-joined or pre-sliced class strings.
pub use crate::cn::{merge, merge_slice};
