-- The clicker's ImGui window: drawn by the lab from the clicker's package
-- (the Vault's "Clicker window" pane), its colours from the app's theme.
-- The style follows the theme by itself; what this script draws in its
-- own colours reads them from `theme`, the clicker's `clicker.*` tokens
-- included, so a theme switch (THEME_SET) recolours all of it.
local ImGui = require 'imgui' '0.10'
local ctx = ImGui.CreateContext('clicker')
local presses = 0

local function loop()
  local visible = ImGui.Begin(ctx, 'Window')
  if visible then
    ImGui.TextColored(ctx, theme.color('clicker.count'),
      presses == 1 and '1 press' or presses .. ' presses')
    ImGui.TextDisabled(ctx, 'theme ' .. theme.id .. ' · ' .. theme.scheme .. ' · revision ' .. theme.revision)
    ImGui.Separator(ctx)
    -- The clicker's own button colour over the theme's, for this button.
    ImGui.PushStyleColor(ctx, ImGui.Col_Button, theme.color('clicker.face'))
    ImGui.PushStyleColor(ctx, ImGui.Col_ButtonHovered, theme.color('clicker.face-hover'))
    if ImGui.Button(ctx, 'Count a press', 140, 28) then presses = presses + 1 end
    ImGui.PopStyleColor(ctx, 2)
    ImGui.ProgressBar(ctx, (presses % 10) / 10, nil, nil, (presses % 10) .. ' / 10')
    ImGui.End(ctx)
  end
  defer(loop)
end
loop()
