-- The ImGui demo window: architect's `ShowDemoWindow`, in Luau.
--
-- Its sections are Dear ImGui's demo's (imgui_demo.cpp, MIT, © Omar
-- Cornut and contributors), in its order: Help, Configuration, Window
-- options, the menu bar, Widgets, Layout & Scrolling, Popups & Modal
-- windows, Tables & Columns, Inputs & Focus. The code and the words are
-- this file's own. A section draws what architect's ImGui can, and says
-- what it cannot in a disabled line, "not implemented: …", naming the API
-- the section needs. The names are looked up as the script runs: a line
-- goes when the module has them.
--
-- architect's ImGui has no scrolling (a window shows what fits) and no
-- collapsing headers, so the sections are buttons that drill in: an open
-- section hides its siblings, and its button, clicked again, goes back.

local ImGui = require 'imgui' '0.10'

-- What the module has: its functions and constants, and each family of
-- constants (`TableFlags_*`) it has any of.
local has = {}
for name in pairs(ImGui) do
  has[name] = true
  local family = string.match(name, '^(%a+)_')
  if family then
    has[family .. '_*'] = true
  end
end

-- About the window's width: architect's ImGui cannot tell a script
-- (GetContentRegionAvail), and the lines below wrap to it.
local WIDTH = 480
-- A label's width, as architect's ImGui measures it.
local function text_width(s)
  return (utf8.len(s) or #s) * 13 * 0.55
end

-- The line a section shows for what it needs and the module lacks.
local function missing(ctx, names)
  local lacking = {}
  for _, name in ipairs(names) do
    if not has[name] then
      table.insert(lacking, name)
    end
  end
  if #lacking == 0 then
    return
  end
  local line = 'not implemented:'
  for i, name in ipairs(lacking) do
    local word = ' ' .. name .. (i < #lacking and ',' or '')
    if text_width(line .. word) > WIDTH then
      ImGui.TextDisabled(ctx, line)
      line = '  '
    end
    line = line .. word
  end
  ImGui.TextDisabled(ctx, line)
end

local _

-- Window options: the flags the demo's window is begun with.
local options = {
  { 'No titlebar', 'WindowFlags_NoTitleBar', false },
  { 'No scrollbar', 'WindowFlags_NoScrollbar', false },
  { 'No move', 'WindowFlags_NoMove', false },
  { 'No resize', 'WindowFlags_NoResize', false },
  { 'No collapse', 'WindowFlags_NoCollapse', false },
  { 'No background', 'WindowFlags_NoBackground', false },
}

local function window_flags()
  local flags = ImGui.WindowFlags_None
  for _, o in ipairs(options) do
    if o[3] then
      flags = flags + ImGui[o[2]]
    end
  end
  return flags
end

local function show_help(ctx)
  ImGui.Text(ctx, 'ABOUT THIS DEMO:')
  ImGui.Text(ctx, '• Its sections show what architect\'s ImGui draws.')
  ImGui.Text(ctx, '• What it cannot draw yet, a section names in a line,')
  ImGui.Text(ctx, '  "not implemented", with the API it needs.')
  ImGui.Separator(ctx)
  ImGui.Text(ctx, 'PROGRAMMER GUIDE:')
  ImGui.Text(ctx, '• The demo is apps/lab/extensions/imgui-demo/demo.luau;')
  ImGui.Text(ctx, '  its sections follow Dear ImGui\'s imgui_demo.cpp.')
  ImGui.Text(ctx, '• A script draws its windows every frame, from Begin to End,')
  ImGui.Text(ctx, '  and keeps its own state: a widget hands back the new value.')
  ImGui.Separator(ctx)
  ImGui.Text(ctx, 'USER GUIDE:')
  ImGui.Text(ctx, '• Click a section to open it, and again to go back.')
  ImGui.Text(ctx, '• Drag a slider; click a checkbox or a button.')
end

local function show_window_options(ctx)
  for i, o in ipairs(options) do
    local column = (i - 1) % 3
    if column > 0 then
      ImGui.SameLine(ctx, column * 160)
    end
    _, o[3] = ImGui.Checkbox(ctx, o[1], o[3])
  end
end

local basic = { clicked = 0, check = true, i1 = 0, f1 = 0.123, element = 0 }
local ELEMENTS = { 'Fire', 'Earth', 'Air', 'Water' }

local function show_basic(ctx)
  if ImGui.Button(ctx, 'Button') then
    basic.clicked = basic.clicked + 1
  end
  if basic.clicked % 2 == 1 then
    ImGui.SameLine(ctx)
    ImGui.Text(ctx, 'Thanks for clicking me!')
  end
  _, basic.check = ImGui.Checkbox(ctx, 'checkbox', basic.check)
  _, basic.i1 = ImGui.SliderInt(ctx, 'slider int', basic.i1, -1, 3)
  _, basic.f1 = ImGui.SliderDouble(ctx, 'slider double', basic.f1, 0, 1, 'ratio = %.3f')
  -- An enum: the format names the value.
  _, basic.element =
    ImGui.SliderInt(ctx, 'slider enum', basic.element, 0, #ELEMENTS - 1, ELEMENTS[basic.element + 1])
end

local function show_text(ctx)
  ImGui.TextColored(ctx, 0xFF00FFFF, 'Pink')
  ImGui.TextColored(ctx, 0xFFFF00FF, 'Yellow')
  ImGui.TextDisabled(ctx, 'Disabled')
  ImGui.Text(ctx, 'UTF-8: “quotes”, café, naïve, Ελληνικά, −40 °C')
end

local progress = 0.42

local function show_progress_bars(ctx)
  -- The demo's bar moves with the time (GetTime); this one, with a slider.
  _, progress = ImGui.SliderDouble(ctx, 'progress', progress, 0, 1, '%.2f')
  ImGui.ProgressBar(ctx, progress)
  ImGui.SameLine(ctx)
  ImGui.Text(ctx, 'Progress Bar')
  ImGui.ProgressBar(ctx, progress, nil, nil, string.format('%d/%d', math.floor(progress * 1753), 1753))
end

local query = { kind = 1, check = false, value = 0.5 }
local KINDS = { 'Text', 'Button', 'Checkbox', 'SliderDouble' }

local function show_querying(ctx)
  _, query.kind = ImGui.SliderInt(ctx, 'Item Type', query.kind, 0, #KINDS - 1, KINDS[query.kind + 1])
  if query.kind == 0 then
    ImGui.Text(ctx, 'ITEM: Text')
  elseif query.kind == 1 then
    ImGui.Button(ctx, 'ITEM: Button')
  elseif query.kind == 2 then
    _, query.check = ImGui.Checkbox(ctx, 'ITEM: Checkbox', query.check)
  else
    _, query.value = ImGui.SliderDouble(ctx, 'ITEM: SliderDouble', query.value, 0, 1)
  end
  local hovered, active = ImGui.IsItemHovered(ctx), ImGui.IsItemActive(ctx)
  ImGui.Text(ctx, 'IsItemHovered() = ' .. tostring(hovered))
  ImGui.Text(ctx, 'IsItemActive() = ' .. tostring(active))
end

local tailor = { false, false, false, false }

local function show_horizontal_layout(ctx)
  ImGui.Text(ctx, 'Two items: Hello')
  ImGui.SameLine(ctx)
  ImGui.TextColored(ctx, 0xFFFF00FF, 'Sailor')
  ImGui.Text(ctx, 'More spacing: Hello')
  ImGui.SameLine(ctx, 0, 20)
  ImGui.TextColored(ctx, 0xFFFF00FF, 'Sailor')
  ImGui.Text(ctx, 'Normal buttons')
  for _, fruit in ipairs({ 'Banana', 'Apple', 'Corniflower' }) do
    ImGui.SameLine(ctx)
    ImGui.Button(ctx, fruit)
  end
  ImGui.Text(ctx, 'Aligned')
  ImGui.SameLine(ctx, 150)
  ImGui.Text(ctx, 'x=150')
  ImGui.SameLine(ctx, 300)
  ImGui.Text(ctx, 'x=300')
  ImGui.Text(ctx, 'Aligned')
  ImGui.SameLine(ctx, 150)
  ImGui.Button(ctx, 'x=150')
  ImGui.SameLine(ctx, 300)
  ImGui.Button(ctx, 'x=300')
  for i, word in ipairs({ 'My', 'Tailor', 'Is', 'Rich' }) do
    if i > 1 then
      ImGui.SameLine(ctx)
    end
    _, tailor[i] = ImGui.Checkbox(ctx, word, tailor[i])
  end
end

-- The sections: each its name, what it draws (`draw`), what it needs and
-- the module may lack (`needs`), and its own sections (`sections`).
local SECTIONS = {
  {
    name = 'Help',
    draw = show_help,
    needs = { 'BulletText', 'CollapsingHeader', 'SeparatorText', 'ShowUserGuide', 'TextLinkOpenURL', 'TreeNode' },
  },
  {
    name = 'Configuration',
    needs = {
      'BeginDisabled', 'CheckboxFlags', 'CollapsingHeader', 'EndDisabled', 'GetTime', 'IsKeyPressed',
      'LogFinish', 'LogText', 'LogToClipboard', 'SeparatorText', 'ShowStyleEditor', 'TreeNode', 'TreePop',
      'ConfigFlags_*', 'Key_*',
    },
  },
  {
    name = 'Window options',
    draw = show_window_options,
    needs = {
      'BeginTable', 'EndTable', 'TableNextColumn', 'GetMainViewport', 'PushItemWidth', 'PopItemWidth',
      'GetContentRegionAvail', 'GetFontSize', 'WindowFlags_MenuBar', 'WindowFlags_NoNav',
      'WindowFlags_NoBringToFrontOnFocus', 'WindowFlags_UnsavedDocument',
    },
  },
  {
    name = 'Menu bar',
    needs = {
      'BeginMenuBar', 'EndMenuBar', 'BeginMenu', 'EndMenu', 'MenuItem', 'ShowMetricsWindow',
      'ShowDebugLogWindow', 'ShowIDStackToolWindow', 'ShowStyleEditor', 'ShowAboutWindow', 'BeginDisabled',
      'EndDisabled', 'DragDouble', 'GetItemRectMax', 'GetItemRectMin', 'GetWindowDrawList',
      'InvisibleButton', 'IsMouseDragging', 'SeparatorText', 'SetItemTooltip', 'SetNextItemShortcut',
      'SetNextItemWidth', 'TreeNode', 'InputFlags_*', 'Key_*', 'SliderFlags_*',
    },
  },
  {
    name = 'Widgets',
    needs = { 'BeginDisabled', 'CollapsingHeader', 'EndDisabled', 'PushItemFlag', 'PopItemFlag', 'TreeNode', 'ItemFlags_*' },
    sections = {
      {
        name = 'Basic',
        draw = show_basic,
        needs = {
          'AlignTextToFramePadding', 'ArrowButton', 'ColorEdit3', 'ColorEdit4', 'Combo', 'DragDouble',
          'DragInt', 'InputDouble', 'InputDouble3', 'InputInt', 'InputText', 'InputTextWithHint', 'LabelText',
          'ListBox', 'PushItemFlag', 'PopItemFlag', 'PushStyleColor', 'PopStyleColor', 'RadioButton',
          'SeparatorText', 'SetItemTooltip', 'SliderAngle', 'TextLinkOpenURL', 'TreeNode', 'TreePop', 'Col_*',
          'Dir_*', 'InputTextFlags_*', 'ItemFlags_*', 'SliderFlags_*',
        },
      },
      { name = 'Bullets', needs = { 'Bullet', 'BulletText', 'SmallButton', 'TreeNode', 'TreePop' } },
      { name = 'Collapsing Headers', needs = { 'CollapsingHeader', 'TreeNode', 'TreePop', 'TreeNodeFlags_*' } },
      {
        name = 'Color',
        needs = {
          'AcceptDragDropPayload', 'BeginDragDropTarget', 'EndDragDropTarget', 'BeginGroup', 'EndGroup',
          'BeginPopup', 'EndPopup', 'OpenPopup', 'CheckboxFlags', 'ColorButton', 'ColorConvertHSVtoRGB',
          'ColorEdit3', 'ColorEdit4', 'ColorPicker3', 'ColorPicker4', 'Combo', 'DragDouble4',
          'GetContentRegionAvail', 'GetFrameHeight', 'SeparatorText', 'SetNextItemWidth', 'TreeNode', 'TreePop',
          'ColorEditFlags_*',
        },
      },
      {
        name = 'Combo',
        needs = {
          'BeginCombo', 'EndCombo', 'CheckboxFlags', 'Combo', 'IsWindowAppearing', 'Selectable', 'SeparatorText',
          'SetItemDefaultFocus', 'SetKeyboardFocusHere', 'SetNextItemShortcut', 'SetNextItemWidth', 'TreeNode',
          'TreePop', 'ComboFlags_*', 'Key_*',
        },
      },
      {
        name = 'Data Types',
        needs = {
          'CheckboxFlags', 'SeparatorText', 'TreeNode', 'TreePop', 'InputTextFlags_*', 'SliderFlags_*',
        },
      },
      { name = 'Disable Blocks', needs = { 'BeginDisabled', 'EndDisabled', 'TreeNode', 'TreePop' } },
      {
        name = 'Drag and Drop',
        needs = {
          'BeginDragDropSource', 'EndDragDropSource', 'SetDragDropPayload', 'BeginDragDropTarget',
          'EndDragDropTarget', 'AcceptDragDropPayload', 'ColorButton', 'ColorEdit3', 'ColorEdit4',
          'GetFrameHeight', 'GetMouseDragDelta', 'ResetMouseDragDelta', 'PushItemFlag', 'PopItemFlag',
          'RadioButton', 'Selectable', 'SetMouseCursor', 'SetTooltip', 'TreeNode', 'TreePop', 'DragDropFlags_*',
          'ItemFlags_*', 'MouseCursor_*',
        },
      },
      {
        name = 'Drag and Slider Flags',
        needs = { 'CheckboxFlags', 'DragDouble', 'DragDouble4', 'DragInt', 'SliderDouble4', 'TreeNode', 'TreePop', 'SliderFlags_*' },
      },
      { name = 'Fonts', needs = { 'ShowFontAtlas', 'TreeNode', 'TreePop' } },
      {
        name = 'Images',
        needs = {
          'GetContentRegionAvail', 'GetWindowDrawList', 'Image', 'ImageButton', 'ImageWithBg', 'NewLine',
          'PushStyleVar', 'PopStyleVar', 'SeparatorText', 'TextWrapped', 'TreeNode', 'TreePop', 'StyleVar_*',
        },
      },
      {
        name = 'List Boxes',
        needs = {
          'BeginListBox', 'EndListBox', 'GetTextLineHeightWithSpacing', 'Selectable', 'SetItemDefaultFocus',
          'TreeNode', 'TreePop', 'ChildFlags_*', 'SelectableFlags_*',
        },
      },
      {
        name = 'Live Edit Flags',
        needs = {
          'BeginDisabled', 'EndDisabled', 'CheckboxFlags', 'Indent', 'Unindent', 'InputInt', 'InputText',
          'TextWrapped', 'TreeNode', 'TreePop', 'ItemFlags_*',
        },
      },
      {
        name = 'Mixed Values',
        needs = {
          'ColorEdit4', 'Combo', 'DragDouble', 'InputDouble', 'PushItemFlag', 'PopItemFlag', 'RadioButton',
          'SeparatorText', 'TreeNode', 'TreePop', 'ItemFlags_*',
        },
      },
      {
        name = 'Multi-component Widgets',
        needs = {
          'CheckboxFlags', 'DragDouble2', 'DragDouble3', 'DragDouble4', 'DragDoubleRange2', 'DragInt2', 'DragInt3',
          'DragInt4', 'DragIntRange2', 'InputDouble2', 'InputDouble3', 'InputDouble4', 'InputInt2', 'InputInt3',
          'InputInt4', 'SeparatorText', 'SliderDouble2', 'SliderDouble3', 'SliderDouble4', 'SliderInt2',
          'SliderInt3', 'SliderInt4', 'TreeNode', 'TreePop', 'SliderFlags_*',
        },
      },
      {
        name = 'Plotting',
        needs = {
          'Combo', 'GetFontSize', 'GetFrameHeight', 'GetTime', 'PlotHistogram', 'PlotLines', 'SeparatorText',
          'SetNextItemWidth', 'TextLinkOpenURL', 'TreeNode', 'TreePop',
        },
      },
      { name = 'Progress Bars', draw = show_progress_bars, needs = { 'GetTime', 'TreeNode', 'TreePop' } },
      {
        name = 'Querying Status',
        draw = show_querying,
        needs = {
          'BeginChild', 'EndChild', 'BeginDisabled', 'EndDisabled', 'BeginPopupContextItem', 'EndPopup',
          'BulletText', 'CheckboxFlags', 'ColorEdit4', 'Combo', 'GetFontSize', 'GetItemRectMax', 'GetItemRectMin',
          'GetItemRectSize', 'InputDouble', 'InputDouble3', 'InputText', 'InputTextMultiline', 'IsItemActivated',
          'IsItemClicked', 'IsItemDeactivated', 'IsItemDeactivatedAfterEdit', 'IsItemEdited', 'IsItemFocused',
          'IsItemToggledOpen', 'IsItemVisible', 'IsWindowFocused', 'IsWindowHovered', 'ListBox', 'MenuItem',
          'PushItemFlag', 'PopItemFlag', 'Selectable', 'SetNextItemOpen', 'TreeNode', 'TreeNodeEx', 'TreePop',
          'ButtonFlags_*', 'ChildFlags_*', 'FocusedFlags_*', 'HoveredFlags_*', 'InputTextFlags_*', 'ItemFlags_*',
          'TreeNodeFlags_*',
        },
      },
      {
        name = 'Selectables',
        needs = {
          'Selectable', 'SelectableFlags_*', 'AlignTextToFramePadding', 'ArrowButton', 'BeginChild', 'EndChild',
          'BeginMultiSelect', 'EndMultiSelect', 'BeginTable', 'EndTable', 'CalcTextSize', 'ColorButton',
          'GetFontSize', 'GetFrameHeight', 'GetFrameHeightWithSpacing', 'GetTextLineHeightWithSpacing', 'GetTime',
          'GetWindowSize', 'IsItemFocused', 'IsKeyPressed', 'IsMouseDoubleClicked', 'NewLine', 'PushStyleVar',
          'PopStyleVar', 'SetNextItemAllowOverlap', 'SetNextItemSelectionUserData', 'SetNextWindowContentSize',
          'SetNextWindowSizeConstraints', 'SmallButton', 'TableNextColumn', 'TableNextRow', 'TableSetColumnIndex',
          'TableSetupColumn', 'TreeNode', 'TreePop', 'WindowFlags_NavFlattened', 'ChildFlags_*', 'ColorEditFlags_*',
          'Dir_*', 'Key_*', 'MultiSelectFlags_*', 'StyleVar_*', 'TableColumnFlags_*', 'TableFlags_*',
        },
      },
      {
        name = 'Selection State & Multi-Select',
        needs = {
          'BeginMultiSelect', 'EndMultiSelect', 'SetNextItemSelectionUserData', 'SetNextItemStorageID',
          'BeginChild', 'EndChild', 'BeginDisabled', 'EndDisabled', 'BeginDragDropSource', 'EndDragDropSource',
          'GetDragDropPayload', 'SetDragDropPayload', 'BeginPopupContextItem', 'EndPopup', 'BeginTable',
          'EndTable', 'TableHeadersRow', 'TableNextColumn', 'TableNextRow', 'TableSetupColumn',
          'TableSetupScrollFreeze', 'BulletText', 'CheckboxFlags', 'ColorButton', 'GetFontSize',
          'GetTextLineHeight', 'GetTextLineHeightWithSpacing', 'InputText', 'IsItemToggledOpen', 'PushStyleVar',
          'PushStyleVarY', 'PopStyleVar', 'RadioButton', 'Selectable', 'SeparatorText', 'SetKeyboardFocusHere',
          'SetNextItemOpen', 'SetNextItemWidth', 'SetNextWindowContentSize', 'Shortcut', 'SmallButton',
          'TextLinkOpenURL', 'TreeNode', 'TreeNodeEx', 'TreeNodeGetOpen', 'TreeNodeSetOpen', 'TreePop',
          'ChildFlags_*', 'ColorEditFlags_*', 'InputFlags_*', 'InputTextFlags_*', 'Key_*', 'MultiSelectFlags_*',
          'SelectableFlags_*', 'StyleVar_*', 'TabBarFlags_*', 'TableColumnFlags_*', 'TableFlags_*',
          'TreeNodeFlags_*',
        },
      },
      {
        name = 'Tabs',
        needs = {
          'BeginTabBar', 'EndTabBar', 'BeginTabItem', 'EndTabItem', 'TabItemButton', 'AlignTextToFramePadding',
          'BeginPopup', 'EndPopup', 'OpenPopup', 'CheckboxFlags', 'Selectable', 'TreeNode', 'TreePop',
          'TabBarFlags_*', 'TabItemFlags_*',
        },
      },
      {
        name = 'Text',
        draw = show_text,
        needs = {
          'TextWrapped', 'PushTextWrapPos', 'PopTextWrapPos', 'PushFont', 'PopFont', 'GetCursorPos',
          'GetCursorScreenPos', 'GetFontSize', 'GetItemRectMax', 'GetItemRectMin', 'GetTextLineHeight',
          'GetWindowDrawList', 'InputText', 'SeparatorText', 'TreeNode', 'TreePop',
        },
      },
      {
        name = 'Text Filter',
        needs = {
          'CreateTextFilter', 'TextFilter_Draw', 'TextFilter_PassFilter', 'BeginChild', 'EndChild',
          'GetTextLineHeightWithSpacing', 'SetNextItemWidth', 'TreeNode', 'TreePop', 'ChildFlags_*',
        },
      },
      {
        name = 'Text Input',
        needs = {
          'InputText', 'InputTextMultiline', 'InputTextWithHint', 'CheckboxFlags', 'GetTextLineHeight', 'TreeNode',
          'TreePop', 'InputTextFlags_*', 'Key_*',
        },
      },
      {
        name = 'Tooltips',
        needs = {
          'BeginTooltip', 'EndTooltip', 'BeginItemTooltip', 'SetTooltip', 'SetItemTooltip', 'BeginDisabled',
          'EndDisabled', 'GetFontSize', 'GetTime', 'PlotLines', 'RadioButton', 'SeparatorText', 'TreeNode',
          'TreePop', 'HoveredFlags_*',
        },
      },
      {
        name = 'Tree Nodes',
        needs = {
          'TreeNode', 'TreeNodeEx', 'TreePop', 'SetNextItemOpen', 'GetTreeNodeToLabelSpacing', 'IsItemClicked',
          'IsItemToggledOpen', 'BeginDragDropSource', 'EndDragDropSource', 'SetDragDropPayload', 'BulletText',
          'CheckboxFlags', 'Indent', 'Unindent', 'SmallButton', 'TextWrapped', 'TreeNodeFlags_*',
        },
      },
      {
        name = 'Vertical Sliders',
        needs = {
          'VSliderDouble', 'VSliderInt', 'BeginGroup', 'EndGroup', 'CollapsingHeader', 'GetFrameHeight',
          'PushStyleColor', 'PopStyleColor', 'PushStyleVar', 'PushStyleVarX', 'PopStyleVar', 'SetNextItemOpen',
          'SetTooltip', 'TreePop', 'Col_*', 'StyleVar_*',
        },
      },
    },
  },
  {
    name = 'Layout & Scrolling',
    sections = {
      {
        name = 'Child windows',
        needs = {
          'BeginChild', 'EndChild', 'BeginMenuBar', 'EndMenuBar', 'BeginMenu', 'EndMenu', 'BeginTable', 'EndTable',
          'TableNextColumn', 'CheckboxFlags', 'DragInt', 'GetContentRegionAvail', 'GetCursorPosX', 'SetCursorPosX',
          'GetFontSize', 'GetItemRectMax', 'GetItemRectMin', 'GetTextLineHeightWithSpacing', 'PushStyleColor',
          'PopStyleColor', 'PushStyleVar', 'PopStyleVar', 'SeparatorText', 'SetNextItemWidth',
          'SetNextWindowSizeConstraints', 'TreeNode', 'TreePop', 'WindowFlags_HorizontalScrollbar',
          'WindowFlags_MenuBar', 'WindowFlags_NoScrollWithMouse', 'ChildFlags_*', 'Col_*', 'StyleVar_*',
          'TableFlags_*',
        },
      },
      {
        name = 'Widgets Width',
        needs = {
          'SetNextItemWidth', 'PushItemWidth', 'PopItemWidth', 'DragDouble', 'GetContentRegionAvail', 'GetFontSize',
          'Indent', 'Unindent', 'TreeNode', 'TreePop',
        },
      },
      {
        name = 'Basic Horizontal Layout',
        draw = show_horizontal_layout,
        needs = {
          'SmallButton', 'AlignTextToFramePadding', 'CalcTextSize', 'Combo', 'Dummy', 'GetContentRegionAvail',
          'GetCursorScreenPos', 'GetItemRectMax', 'ListBox', 'PushItemWidth', 'PopItemWidth', 'SetItemTooltip',
          'TextWrapped', 'TreeNode', 'TreePop',
        },
      },
      {
        name = 'Groups',
        needs = {
          'BeginGroup', 'EndGroup', 'BeginListBox', 'EndListBox', 'GetItemRectSize', 'PlotHistogram', 'Selectable',
          'SetItemTooltip', 'TreeNode', 'TreePop',
        },
      },
      {
        name = 'Text Baseline Alignment',
        needs = {
          'AlignTextToFramePadding', 'BulletText', 'GetFontSize', 'Indent', 'Unindent', 'PushStyleVarY',
          'PopStyleVar', 'SmallButton', 'TreeNode', 'TreeNodeEx', 'TreePop', 'StyleVar_*', 'TreeNodeFlags_*',
        },
      },
      {
        name = 'Scrolling',
        needs = {
          'GetScrollX', 'GetScrollY', 'GetScrollMaxX', 'GetScrollMaxY', 'SetScrollX', 'SetScrollY',
          'SetScrollHereX', 'SetScrollHereY', 'SetScrollFromPosX', 'SetScrollFromPosY', 'BeginChild', 'EndChild',
          'BeginGroup', 'EndGroup', 'BeginMenuBar', 'EndMenuBar', 'BeginTabBar', 'EndTabBar', 'BeginTabItem',
          'EndTabItem', 'BeginTable', 'EndTable', 'TableNextColumn', 'CalcTextSize', 'CollapsingHeader', 'Columns',
          'NextColumn', 'GetColumnWidth', 'DragDouble', 'DragInt', 'Dummy', 'GetContentRegionAvail',
          'GetCursorScreenPos', 'GetCursorStartPos', 'GetFontSize', 'GetFrameHeightWithSpacing', 'GetID',
          'GetTextLineHeight', 'GetWindowDrawList', 'PushItemWidth', 'PopItemWidth', 'PushStyleColor',
          'PopStyleColor', 'PushStyleVar', 'PopStyleVar', 'SetNextItemWidth', 'SetNextWindowContentSize',
          'SmallButton', 'TextWrapped', 'TreeNode', 'TreePop', 'WindowFlags_AlwaysVerticalScrollbar',
          'WindowFlags_HorizontalScrollbar', 'WindowFlags_MenuBar', 'ChildFlags_*', 'Col_*', 'StyleVar_*',
          'TableFlags_*',
        },
      },
      {
        name = 'Text Clipping',
        needs = {
          'PushClipRect', 'PopClipRect', 'DragDouble2', 'GetFontSize', 'GetItemRectMax', 'GetItemRectMin',
          'GetWindowDrawList', 'InvisibleButton', 'IsItemVisible', 'IsMouseDragging', 'TextWrapped', 'TreeNode',
          'TreePop', 'MouseButton_*',
        },
      },
      {
        name = 'Overlap Mode',
        needs = {
          'SetNextItemAllowOverlap', 'GetCursorScreenPos', 'SetCursorScreenPos', 'BeginPopup', 'OpenPopup',
          'CollapsingHeader', 'Selectable', 'SmallButton', 'TreeNode', 'TreePop', 'HoveredFlags_*',
          'SelectableFlags_*',
        },
      },
    },
  },
  {
    name = 'Popups & Modal windows',
    sections = {
      {
        name = 'Popups',
        needs = {
          'BeginPopup', 'EndPopup', 'OpenPopup', 'BeginMenuBar', 'EndMenuBar', 'BeginMenu', 'EndMenu', 'MenuItem',
          'Selectable', 'SeparatorText', 'SetItemTooltip', 'TextWrapped', 'TreeNode', 'TreePop',
          'WindowFlags_MenuBar',
        },
      },
      {
        name = 'Context menus',
        needs = {
          'BeginPopupContextItem', 'OpenPopupOnItemClick', 'CloseCurrentPopup', 'EndPopup', 'OpenPopup',
          'DragDouble', 'InputText', 'Selectable', 'SetItemTooltip', 'SetNextItemWidth', 'TreeNode', 'TreePop',
          'MouseButton_*', 'PopupFlags_*',
        },
      },
      {
        name = 'Modals',
        needs = {
          'BeginPopupModal', 'CloseCurrentPopup', 'EndPopup', 'OpenPopup', 'BeginMenuBar', 'EndMenuBar',
          'BeginMenu', 'EndMenu', 'MenuItem', 'ColorEdit4', 'Combo', 'GetMainViewport', 'PushStyleVar',
          'PopStyleVar', 'SetItemDefaultFocus', 'TextWrapped', 'TreeNode', 'TreePop', 'WindowFlags_MenuBar',
          'Col_*', 'StyleVar_*',
        },
      },
      {
        name = 'Menus inside a regular window',
        needs = {
          'BeginMenu', 'EndMenu', 'MenuItem', 'BeginCombo', 'EndCombo', 'BeginItemTooltip', 'EndTooltip',
          'CalcTextSize', 'CheckboxFlags', 'CollapsingHeader', 'GetCursorPosX', 'SetCursorPosX', 'GetFontSize',
          'GetTextLineHeightWithSpacing', 'PushStyleVar', 'PushStyleVarY', 'PopStyleVar', 'PushTextWrapPos',
          'PopTextWrapPos', 'Selectable', 'SetNextItemOpen', 'TableGetSortSpecs', 'TextWrapped', 'TreeNode',
          'TreePop', 'Col_*', 'SortDirection_*', 'StyleVar_*', 'TableColumnFlags_*', 'TableFlags_*',
        },
      },
    },
  },
  {
    name = 'Tables & Columns',
    sections = {
      {
        name = 'Tables',
        needs = {
          'BeginTable', 'EndTable', 'TableNextRow', 'TableNextColumn', 'TableSetColumnIndex', 'TableSetupColumn',
          'TableSetupScrollFreeze', 'TableHeadersRow', 'TableHeader', 'TableAngledHeadersRow', 'TableGetSortSpecs',
          'TableGetColumnFlags', 'TableGetColumnIndex', 'TableGetColumnName', 'TableGetRowIndex',
          'TableSetBgColor', 'AlignTextToFramePadding', 'BeginDisabled', 'EndDisabled', 'BeginPopup',
          'BeginPopupContextItem', 'EndPopup', 'OpenPopup', 'CloseCurrentPopup', 'CheckboxFlags',
          'CollapsingHeader', 'ColorButton', 'Combo', 'DragDouble', 'DragDouble2', 'DragInt', 'GetColorU32',
          'GetContentRegionAvail', 'GetFontSize', 'GetFrameHeight', 'GetScrollMaxX', 'GetScrollMaxY',
          'GetScrollX', 'GetScrollY', 'GetTextLineHeightWithSpacing', 'GetWindowDrawList', 'Indent', 'Unindent',
          'InputText', 'IsAnyItemHovered', 'IsItemDeactivated', 'IsMouseReleased', 'PushItemWidth',
          'PopItemWidth', 'PushStyleColor', 'PopStyleColor', 'PushStyleVar', 'PushStyleVarY', 'PopStyleVar',
          'RadioButton', 'Selectable', 'SetNextItemOpen', 'SetNextItemWidth', 'SliderAngle', 'SliderDouble2',
          'SmallButton', 'TextWrapped', 'TreeNode', 'TreeNodeEx', 'TreePop', 'Col_*', 'ColorEditFlags_*',
          'PopupFlags_*', 'SelectableFlags_*', 'SliderFlags_*', 'StyleVar_*', 'TableColumnFlags_*',
          'TableFlags_*', 'TableRowFlags_*', 'TreeNodeFlags_*',
        },
      },
      {
        name = 'Columns (legacy API)',
        needs = {
          'Columns', 'NextColumn', 'GetColumnIndex', 'GetColumnOffset', 'GetColumnWidth', 'BeginChild', 'EndChild',
          'CollapsingHeader', 'DragInt', 'GetContentRegionAvail', 'GetFontSize', 'InputDouble', 'Selectable',
          'SetNextItemOpen', 'SetNextItemWidth', 'SetNextWindowContentSize', 'TextWrapped', 'TreeNode', 'TreePop',
          'WindowFlags_HorizontalScrollbar', 'ChildFlags_*', 'SelectableFlags_*',
        },
      },
    },
  },
  {
    name = 'Inputs & Focus',
    sections = {
      {
        name = 'Inputs',
        needs = {
          'IsKeyDown', 'IsMouseDown', 'IsMousePosValid', 'GetKeyName', 'SetNextItemOpen', 'TreeNode', 'TreePop',
          'Key_*',
        },
      },
      {
        name = 'Outputs',
        needs = {
          'SetNextFrameWantCaptureKeyboard', 'SetNextFrameWantCaptureMouse', 'ColorButton', 'GetFontSize',
          'GetFrameHeight', 'SetNextItemWidth', 'TreeNode', 'TreePop', 'ColorEditFlags_*', 'InputFlags_*', 'Key_*',
          'SliderFlags_*',
        },
      },
      {
        name = 'Shortcuts',
        needs = {
          'Shortcut', 'SetNextItemShortcut', 'GetItemID', 'BeginChild', 'EndChild', 'BeginDisabled', 'EndDisabled',
          'BeginPopup', 'EndPopup', 'OpenPopup', 'CheckboxFlags', 'GetTextLineHeightWithSpacing', 'Indent',
          'Unindent', 'InputText', 'IsWindowFocused', 'PushStyleColor', 'PopStyleColor', 'RadioButton',
          'SeparatorText', 'TreeNode', 'TreePop', 'Col_*', 'InputFlags_*', 'InputTextFlags_*', 'Key_*',
        },
      },
      {
        name = 'Mouse Cursors',
        needs = {
          'GetMouseCursor', 'SetMouseCursor', 'BeginDisabled', 'EndDisabled', 'Bullet', 'CheckboxFlags',
          'Selectable', 'TreeNode', 'TreePop', 'MouseCursor_*',
        },
      },
      {
        name = 'Tabbing',
        needs = { 'InputText', 'PushItemFlag', 'PopItemFlag', 'TreeNode', 'TreePop', 'ItemFlags_*' },
      },
      {
        name = 'Focus from code',
        needs = {
          'SetKeyboardFocusHere', 'InputText', 'PushItemFlag', 'PopItemFlag', 'SliderDouble3', 'TextWrapped',
          'TreeNode', 'TreePop', 'ItemFlags_*',
        },
      },
      {
        name = 'Dragging',
        needs = {
          'IsMouseDragging', 'GetMouseDragDelta', 'GetForegroundDrawList', 'GetColorU32', 'TextWrapped', 'TreeNode',
          'TreePop', 'Col_*',
        },
      },
    },
  },
}

-- The sections open, outermost first.
local open = {}

-- Section buttons, as many a row as fit.
local function section_buttons(ctx, sections, prefix)
  local right = 0
  for _, s in ipairs(sections) do
    local label = '▸ ' .. s.name
    local w = text_width(label) + 12
    if right > 0 and right + 8 + w <= WIDTH then
      ImGui.SameLine(ctx)
      right = right + 8 + w
    else
      right = w
    end
    if ImGui.Button(ctx, label .. '###' .. prefix .. s.name) then
      table.insert(open, s)
    end
  end
end

-- The open sections, a row of buttons that go back to each; the last of
-- them drawn, and its own sections to open. None open, the top ones.
local function show_sections(ctx)
  local prefix = ''
  for i, s in ipairs(open) do
    if i > 1 then
      ImGui.SameLine(ctx)
    end
    if ImGui.Button(ctx, '▾ ' .. s.name .. '###open/' .. prefix .. s.name) then
      for d = #open, i, -1 do
        open[d] = nil
      end
      break
    end
    prefix = prefix .. s.name .. '/'
  end
  local here = open[#open]
  if here == nil then
    ImGui.TextDisabled(ctx, 'Click a section to open it.')
    section_buttons(ctx, SECTIONS, '')
    return
  end
  ImGui.Separator(ctx)
  if here.draw then
    here.draw(ctx)
  end
  if here.needs then
    missing(ctx, here.needs)
  end
  if here.sections then
    ImGui.Spacing(ctx)
    section_buttons(ctx, here.sections, prefix)
  end
end

-- `ShowDemoWindow(ctx, open?)`: the demo's window, a close button when
-- `open` is given; answers whether it is still open.
local function ShowDemoWindow(ctx, p_open)
  ImGui.SetNextWindowSize(ctx, 550, 680, ImGui.Cond_FirstUseEver)
  local visible
  visible, p_open = ImGui.Begin(ctx, 'Dear ImGui Demo', p_open, window_flags())
  if visible then
    ImGui.Text(ctx, "architect's ImGui says hello, from Luau.")
    ImGui.Spacing(ctx)
    show_sections(ctx)
    ImGui.End(ctx)
  end
  return p_open
end

local ctx = ImGui.CreateContext('imgui-demo')

local function loop()
  ShowDemoWindow(ctx)
  defer(loop)
end

loop()
