extension = {
  id = "injection-test",
  name = "Injection Test Button",
  version = "0.1.0",
  description = "Minimal CEF injection test — adds a visible floating button to Steam Store.",

  detect = function(hostPath)
    return { status = "installed" }
  end,

  install = function(hostPath)
    return { success = true }
  end,

  enable = function(hostPath)
    return { success = true }
  end,

  disable = function(hostPath)
    return { success = true }
  end,

  uninstall = function(hostPath)
    return { success = true }
  end,
}
