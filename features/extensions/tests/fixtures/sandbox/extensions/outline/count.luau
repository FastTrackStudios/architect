-- The count widget: how many list items the note has, from the widget's
-- props (`embed().props`).
render(function()
  local e = embed()
  local n = 0
  if e and e.props and e.props.items then
    n = e.props.items
  end
  return div():attr("id", "outline-count"):class("outline-count"):child(tostring(n) .. " list items")
end)
