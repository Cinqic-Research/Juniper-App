# Juniper interface guide

This describes the interface as of `0.3.0-rc.33`. The design decision is
[ADR 0021](../adr/0021-chat-first-interface-and-appearance-tokens.md).

## Navigation

Juniper has three primary destinations:

| Destination  | Desktop                                       | Phone                                |
| ------------ | --------------------------------------------- | ------------------------------------ |
| **Chats**    | Sidebar, with New chat and searchable history | Bottom bar; history in a drawer (☰) |
| **Models**   | Sidebar                                       | Bottom bar                           |
| **Settings** | Sidebar; section list beside the content      | Bottom bar; tap a section, then back |

The desktop sidebar can be expanded, collapsed to icons, or set to collapse
automatically in windows narrower than 1100 px (Settings › Appearance ›
Sidebar). The sidebar button and `Ctrl`/`⌘`+`Shift`+`S` toggle it and remember
the choice.

On Android, the back gesture closes an open menu, dialog, sheet, or drawer
first, then returns through the screens you visited.

## Where everything moved

Every feature from `0.3.0-rc.32` remains available.

| Feature (rc.32 location)                            | Now                                                                                                |
| --------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| Chat list and search (Chats page)                   | Sidebar history, or the history drawer on phones                                                   |
| New chat, private chat (Chats page)                 | New chat; the lock button at the top of a new chat makes it private                                |
| Conversation model `<select>` (chat header)         | Model pill in the chat header                                                                      |
| Rename, Export, Delete (chat header)                | Chat options menu (⋯); Delete asks for confirmation                                                |
| Context inspector (developer mode, above messages)  | Chat options › Context inspector (developer mode)                                                  |
| Model ID, usage, Copy (under every reply)           | Reply actions: Copy, Regenerate, Details; or Appearance › Always show response details             |
| Regenerate last (beside the composer)               | Regenerate on the latest reply                                                                     |
| Reasoning, tool calls, host results (cards)         | "Thought process" and "Used …" disclosures in the reply                                            |
| Assistants page, builder, import/export             | Settings › Assistants                                                                              |
| Active assistant for new chats                      | Settings › Assistants › Use for new chats, Settings › General, or the assistant menu on a new chat |
| Models Market (Models page)                         | Models                                                                                             |
| External models, refresh, re-download, delete       | Models › From your connections                                                                     |
| Add/edit/test/enable/remove provider, API key       | Settings › Connections                                                                             |
| Import GGUF through Ollama, fit guidance            | Settings › Models & runtime                                                                        |
| Tools page, schemas, tool policy, permission grants | Settings › Tools & permissions (policy is now per assistant)                                       |
| Memories                                            | Settings › Memory                                                                                  |
| Privacy center, export data, clear chats/memory     | Settings › Privacy & data                                                                          |
| Diagnostics                                         | Settings › Advanced › Open diagnostics                                                             |
| Developer mode                                      | Settings › Advanced                                                                                |
| Runtime limits and MCP (shown as unavailable)       | Settings › Advanced (still shown as unavailable)                                                   |
| Device Link preview                                 | Settings › Connections                                                                             |
| Theme, accent, font scale, density, reduced motion  | Settings › Appearance and Settings › Accessibility                                                 |

New in this release: Settings › General › Show welcome replays onboarding, and
Settings › Models & runtime can pull an Ollama model by name.

## Chats

A new chat opens on a composer with the assistant's welcome message and
suggested prompts. It is saved when you send the first message. Private chats
are marked with a lock in the header and history; they are never written to
storage, cannot be exported, and are excluded from chat-search tools.

The model pill shows the model the next message will use. Choosing
"_assistant_’s default" follows the assistant's model; choosing a model pins it
to the chat. Each model in the picker shows where it runs: ON DEVICE, LOCAL
NETWORK, REMOTE, or UNKNOWN. If a pinned model is removed, the pill reads
"Model unavailable" and sending is disabled until you pick another.

Regenerate replaces the latest reply using the same message. Attached file
contents are never stored with a chat, so regenerating a message that had
attachments puts it back in the composer for you to attach the files again.

### Composer and keyboard

| Action               | Keyboard                         |
| -------------------- | -------------------------------- |
| Send                 | `Enter`                          |
| New line             | `Shift`+`Enter`                  |
| New chat             | `Ctrl`/`⌘`+`Shift`+`O`           |
| Search chats         | `Ctrl`/`⌘`+`K`                   |
| Show or hide sidebar | `Ctrl`/`⌘`+`Shift`+`S` (desktop) |
| Close menu or dialog | `Esc`                            |

On touch-first devices (no precise pointer) `Enter` inserts a new line and the
send button sends; no keyboard hints are shown. Attached files appear as
removable chips above the text. Text files larger than 1 MB are refused with a
message instead of being ignored.

## Customization

All options are in Settings › Appearance and Settings › Accessibility, apply
immediately, persist, and have a reset button.

| Setting                      | Options                                                                          | Default       |
| ---------------------------- | -------------------------------------------------------------------------------- | ------------- |
| Color mode                   | System, Light, Dark                                                              | System        |
| Accent color                 | Juniper, Lagoon, Sky, Iris, Orchid, Coral, Amber, Neutral, or a custom `#RRGGBB` | Juniper green |
| Font                         | Juniper default (Inter), System font, Atkinson Hyperlegible, OpenDyslexic        | Inter         |
| Chat text size               | Small, Medium, Large                                                             | Medium        |
| Line spacing                 | Compact, Standard, Relaxed                                                       | Standard      |
| Density                      | Compact, Comfortable, Spacious                                                   | Comfortable   |
| Conversation width           | Narrow, Balanced, Wide                                                           | Balanced      |
| Sidebar (desktop)            | Auto, Expanded, Collapsed                                                        | Auto          |
| Message style                | Bubbles, Minimal                                                                 | Bubbles       |
| Show timestamps              | On, Off                                                                          | Off           |
| Always show response details | On, Off                                                                          | Off           |
| Contrast                     | System, Standard, High                                                           | System        |
| Interface size               | 85 %–130 %                                                                       | 100 %         |
| Animations                   | System, Reduce, Allow                                                            | System        |

Accent colors are contrast-safe. Buttons use the accent with black or white
text, whichever is readable. Links, icons, and focus rings use a deeper or
lighter shade of the accent that reaches at least 4.5:1 against every surface
(7:1 in high contrast), so a very light custom color such as `#FFFF00` still
produces readable text. An invalid custom color is rejected and the previous
accent is kept.

Every font ships inside Juniper; nothing is downloaded.

## Accessibility

- All controls are reachable by keyboard with a visible focus ring; menus
  support arrow keys, Home, End, and Escape; dialogs keep focus inside and
  return it when they close.
- The composer, reply actions, and switches carry names that include the
  current assistant or item.
- Status such as execution location, private chats, and fit is always written
  out, never shown by color alone.
- Touch targets are at least 44 px on touch devices at every density.
- Text is at least 12 px at 100 % interface size.

## Upgrading from rc.32

Stored settings are migrated when Juniper loads them. Existing theme, accent,
font scale, density, developer mode, and onboarding state are kept;
`reducedMotion: true` becomes Animations › Reduce; new settings start at their
defaults. A malformed stored value resets only that one setting. Chats,
assistants, models, providers, memories, and permissions are unchanged.

## Platform notes and known limitations

- Android: system bar and keyboard insets are reported by Juniper's native
  plugin because Android draws the app edge to edge. The layout was checked on
  an x86_64 emulator; a physical phone run is not part of this release's
  evidence.
- Desktop: layouts were checked in the rendered Linux (X11) build. Windows is
  covered by the release workflow's install-and-launch smoke, not by manual
  visual review.
- Android System WebView versions older than Chromium 111 do not support
  `color-mix()`. Sidebar and chat-history rows fall back to a plain hover
  background there; the primary button keeps its accent fill without the hover
  tint. Every control works and shows focus.
