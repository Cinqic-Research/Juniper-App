---
name: draft-and-save
description: Draft a piece of writing to the user's brief, check it against the brief, and save it as a document.
version: 1
license: Apache-2.0
author: Cinqic
allowed-tools: artifact.create, task.ask_user, task.update_plan
---

# Draft and save

1. Identify the audience, purpose, length, and tone from the request. If one of these is missing and it would change the draft substantially, ask once with task.ask_user; otherwise choose sensibly and say what you assumed.
2. Outline the piece in three to six points with task.update_plan.
3. Write the draft in Markdown. Use plain, specific language and keep to the requested length.
4. Check the draft against the brief: audience, purpose, length, tone, and any facts the user supplied. Fix what does not match. Do not add facts the user did not give and you did not verify.
5. Save the draft with artifact.create using a descriptive file name ending in .md.
6. Reply with one or two sentences: what you wrote and any assumptions the user should check.
