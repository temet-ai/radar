---
name: ask
version: 2
inputs: one or more items to ask about (a message, a change request, a ticket, a branch, a file and line), and optionally the operator's own question
scope: the working directory of this session; the items given; read-only questions about them
permissions: read files and run read-only git commands (status, diff, log, show) in the working directory; ask before any other command; no file writes; no posting to any service
expected-output: a direct answer in plain language, naming the files and lines or the items it rests on, and saying what could not be established
stopping-point: the question is answered; no further work and no changes
---
Tell me what the item below is about and what, if anything, needs doing about it. Lead with the answer and keep it short. If the answer depends on the code, look in the working directory and say which files you read. If the item does not say enough, say what is missing.
