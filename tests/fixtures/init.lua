-- Test configuration for rexia's live tests.
rex.bind("ctrl+a>c", "client.tab.new")
rex.bind("ctrl+a>z", "pane.zoom")
rex.bind("ctrl+a>,", "client.tab.rename")
rex.bind("ctrl+a>shift+4", "session.rename")
rex.mode("resize", { exclusive = true })
rex.bind("ctrl+alt+r", "client.mode.enter", { name = "resize" })
rex.bind("resize/left", "pane.resize", { direction = "left" })
rex.bind("resize/enter", "client.mode.exit")
rex.bind("ctrl+shift+o", "pane.split", { direction = "down" })
rex.bind("ctrl+shift+u", function(ctx)
  rex.block.call("com.superlogical.terminal", "write", { data = "echo rexia-lua-key\r" })
  return true
end)
rex.action{
  name = "rexia_marker",
  title = "Write a Marker",
  run = function(ctx)
    rex.block.call("com.superlogical.terminal", "write", { data = "echo rexia-lua-action\r" })
  end,
}
