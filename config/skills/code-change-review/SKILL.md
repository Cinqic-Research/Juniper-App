---
name: code-change-review
description: Review a proposed code change against the project's files and report concrete problems with evidence, running the project's checks where the sandbox allows.
version: 1
license: Apache-2.0
author: Cinqic
allowed-tools: workspace.list, workspace.read, workspace.search, artifact.read, code.run, artifact.create
---

# Code change review

1. Find the change: a patch or document named in the request (artifact.read) or the files the user pointed to (workspace.read).
2. Read the surrounding code the change touches. Use workspace.search to find callers and tests.
3. If code.run is available, run the project's existing test or check command once (for example python3 -m pytest -q, or node with the project's test script). Report the actual output. If it is not available, say that no checks were run.
4. List findings in order of severity. For each: file and line, what goes wrong, a concrete input or situation that triggers it, and a suggested fix. Do not report style preferences as defects.
5. Say what you did not check.
6. You are reviewing, not editing: do not change files. Save the review with artifact.create (review.md) and reply with the number of findings and the most serious one.
