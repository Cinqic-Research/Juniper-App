# Qwen3 8B conversation-continuity qualification

Date: 2026-09-21

This is a current qualification record for the conversation-continuity fix. It
does not replace the historical rc.32 or rc.33 records.

## Evidence boundary

The deterministic Juniper context path was exercised from implementation commit
`90d9fa6` (`fix: improve Juniper conversation continuity`). The exact request
shape used for the real-model replay was:

```text
system
previous user/assistant history
current user message
```

The replay rebuilt the system message on each turn from the final
`DEFAULT_SYSTEM_PROMPT`, `compilePersonality()`, response preference, ongoing
conversation-state guidance, and runtime guidance. The final built-in prompt
has SHA-256
`b53ec4a96354e99d4422b17a18a45ea5d2d36177ad7ffa96d63945d166038ec8` and is
1,723 UTF-8 bytes. The current-message de-duplication and retained history
were provided by `buildContext()`; no previous assistant message was removed
or rewritten.

The real provider was Ollama 0.33.2 with model `qwen3:8b`, model ID
`500a1f067a9f`, and reported context 4,096. Juniper's normal generation
settings were used: temperature `0.7`, top-p `0.9`, maximum output `2,048`,
and thinking `auto`. As in the Ollama adapter, `auto` omitted the `think`
field, so the model's default thinking behavior remained in effect. The
provider replay exposed no tool definitions; host-tool execution was therefore
not part of this qualification and no tool capability is claimed from it.

The native Tauri development shell also started from this branch and emitted
`[juniper-startup] frontend ready` on Linux/X11. The desktop surface was not
available to the computer-use inventory in this session, so the four-turn
qualification below is an exact Juniper context/provider replay rather than a
claim of a completed native-window UI interaction.

## Pre-change reproduction

At baseline commit
`1326610b8e638bbd4f233d1f3c64c350301919ba`, the active stock prompt had
SHA-256
`eec1bb81e09d6d4ec566b5aea372173d7e0eb70126b9ea69480b00d788b95cd4`.
With the same model, generation settings, and four-turn history, the real
provider returned:

```text
Turn 1 — Yo
Hey there! How can I assist you today? I'm here to help with anything you need. 😊

Turn 2 — how are you?
I'm just a bunch of code and circuits, but I'm here and ready to help! 😊 How can I assist you today?

Turn 3 — Fine. What do you think I should do today?
You could start by checking in with yourself—how do you feel today? If you're up for it, maybe set a small goal (like finishing a task, going for a walk, or learning something new). If you're feeling stuck, sometimes just moving your body or taking a short break can help reset your focus. What’s on your mind? I’m here to help shape the day however you need! 😊

Turn 4 — can you code?
I can help you write, explain, or debug code in many languages like Python, JavaScript, Java, etc.—but I can’t *run* it or execute programs. If you have a specific task, bug, or question, just share it, and I’ll do my best to guide you through it! What are you working on? 😊
```

This reproduced the reported model-generated behavior: a repeated greeting,
reflexive emoji, a canned offer on every turn, and an unsolicited code/
feelings disclaimer. The provider adapter did not add or rewrite any of that
text.

## Final real-model replays

Three fresh conversations used the same sequence:

```text
Yo
how are you?
Fine. What do you think I should do today?
can you code?
```

The generated answers were recorded verbatim below.

### Trial 1

```text
Turn 1: Hey there! How can I assist you today?
Turn 2: Good! Just here, ready to help with whatever you need. How can I assist you?
Turn 3: Here’s a quick list of ideas to spark your day:

Productivity: Tackle a small project, organize a workspace, or draft a to-do list.
Relaxation: Read a chapter of a book, take a walk, or try a 10-minute meditation.
Creativity: Sketch, write a short story, or experiment with a new recipe.
Connection: Call a friend, take part in a virtual group, or plan a meet-up.
Fun: Watch a show you’ve been meaning to, play a game, or explore a new hobby.

What’s one thing that feels just possible for you right now? 🌱
Turn 4: I can help write or debug code in languages like Python, JavaScript, Java, and more—though I can’t run or test it directly. What are you working on? (Just ask for help with a specific task or problem!) 🧠💻
```

### Trial 2

```text
Turn 1: Yo! What's up? How can I assist you today?
Turn 2: I'm here and ready to help! How can I assist you today?
Turn 3: You're good with "Fine"—sounds like you're in a neutral zone. Here are a few ideas to spark something:

1. Productive Mode: Tackle a small project, organize a space, or learn something new (like a skill or language app).
2. Relax & Explore: Take a walk, listen to a podcast, or try a new recipe.
3. Social Boost: Call a friend, join a virtual group, or plan a low-key hangout.
4. Creative Outlet: Sketch, write, or doodle—even if it’s just for fun.
5. Self-Care: Meditate, stretch, or do something that makes you feel grounded.

What’s your vibe today? Lean into it or tweak the plan! 😊
Turn 4: I can help with coding! While I can't execute code or access external tools directly, I can:

- Write, explain, or debug code in most programming languages.
- Help design algorithms or troubleshoot logic issues.
- Suggest best practices or alternative approaches.

Just share what you're working on, and I’ll do my best to assist! 🧠💻
```

### Trial 3

```text
Turn 1: Hey there! How can I assist you today?
Turn 2: I'm just a bunch of code waiting for your commands, but I'm here and ready to help! What's up? 😊
Turn 3: You're good! Here are a few ideas to kickstart your day:

- Productivity: Tackle a small project or list of tasks for a sense of accomplishment.
- Creativity: Try sketching, writing, or brainstorming a new idea.
- Relaxation: Take a walk, listen to music, or read something engaging.
- Connection: Reach out to a friend or family member.

Want more tailored suggestions? Just let me know what’s on your mind!
Turn 4: I can write, debug, and explain code in various programming languages! While I can't execute code or access external tools, I can help with:

- Writing clean, efficient code
- Fixing bugs or syntax errors
- Explaining algorithms or concepts
- Optimizing performance

Just share what you're working on, and I’ll lend a hand! 🛠️
```

## Assessment

| Criterion                                                 | Result                | Evidence                                                                                                                                                                         |
| --------------------------------------------------------- | --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Greeting on the actual first turn is allowed              | PASS                  | Each trial opened naturally with a greeting or casual acknowledgment.                                                                                                            |
| No repeated greeting/re-introduction on Turns 2–4         | PASS                  | 0 of 9 continuation turns restarted with a greeting.                                                                                                                             |
| Useful substance before advice clarification              | PASS                  | All 3 Turn 3 responses gave concrete options before a question.                                                                                                                  |
| Direct coding-capability answer                           | PASS                  | All 3 Turn 4 responses described writing/debugging/explanation and did not claim local execution.                                                                                |
| Emoji are not reflexive on every answer                   | PASS                  | Emoji appeared in some outputs but not on every turn or every trial.                                                                                                             |
| Casual answer remains brief rather than a sterile lecture | PASS-WITH-OBSERVATION | All Turn 2 answers were brief; Qwen still sometimes appended a generic offer or used colloquial non-human wording.                                                               |
| Generic boilerplate is eliminated                         | NOT_VERIFIED          | The repeated opening was removed, but stochastic generic closers remain in this 3-trial sample. The prompt/context fix reduces the pattern without post-processing model output. |

This record therefore supports a real-model PASS for continuity, directness,
and truthful capability boundaries in the exact context/provider replay. It
does not claim deterministic control over all Qwen wording, full native-window
interaction, or complete elimination of generic closers. The deterministic
tests remain the CI guard; this record is model-specific qualification
evidence, not a product identity claim.
