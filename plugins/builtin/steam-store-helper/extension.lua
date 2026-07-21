extension = {
  id = "steam-store-helper",
  name = "Steam Store Package Helper",
  version = "1.0.0",
  description = "Injects asset download controls into the Steam Store action bar via CDP WebSocket.",

  -- The inject.js payload lives as a sibling file on disk.
  -- The TS bridge reads it and pushes it via inject_to_steam_tab.
  -- This Lua script only handles lifecycle acknowledgement.

  detect = function(hostPath)
    -- No on-disk artifacts to check; extension is present by definition.
    -- The TS bridge monitors Steam CEF tab state separately.
    return { status = "installed" }
  end,

  install = function(hostPath)
    -- All assets are bundled; nothing extra to write to disk.
    return { success = true }
  end,

  enable = function(hostPath)
    -- Return the relative path to the inject payload so the TS bridge
    -- can read it and dispatch it through inject_to_steam_tab.
    return {
      success = true,
      injectScript = "inject.js",
      cefEndpoint = "http://127.0.0.1:8080/json",
      targetUrl = "/app/"
    }
  end,

  disable = function(hostPath)
    -- No persistent state to clean up; JS injection stops when the
    -- TS bridge stops polling.
    return { success = true }
  end,

  uninstall = function(hostPath)
    -- AppData directory deletion (handled by the TS cascading
    -- uninstall) is sufficient. No Steam-side artifacts remain.
    return { success = true }
  end,
}
