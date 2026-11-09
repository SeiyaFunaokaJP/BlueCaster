---
title: Clean-room policy (developer note)
layout: default
nav_exclude: true
english_only: true
---

# Clean-room policy

BlueCaster is Apache-2.0 / MIT. Sometimes the only description of what a
chip expects is GPL-licensed source. This page sets the rules for learning
from such sources without letting anything from them into BlueCaster.

## What may be learned

Only observable facts about the hardware or the wire: which interface or
endpoint carries what, command codes, sizes, the order of steps, what the
chip answers. Each fact is written down in our own words, with its source
noted, and marked as a fact, as a design choice of that source, or as our
own hypothesis.

## What may never be taken

- code, in any language, including a translation
- structure: function layout, state machines, names, the split into files
- comments or other text, pasted or paraphrased line by line

## Fact sheets stay outside the repository

Fact sheets are kept privately, outside this repository, and are never
committed. Nothing in the repository (code, comments, documentation,
commit messages, pull requests, release notes) describes or cites such a
source. The code is written from the fact sheet, never from the source.

## Vendor values and undocumented commands

- Vendor-specific values (register addresses, magic values, undocumented
  command codes) are never in the code. The tester types them in under
  Developer > Debug.
- An undocumented command or register write stays isolated in its own
  module, opt-in (off while its field is empty), limited to the vendor it
  is for, and reversible (clear the field, then restart or reboot).
- Logs from these experiments carry only results and counts, never
  addresses, keys or device paths.

## Recording it

Whoever works from a fact sheet says so in the pull request, naming the
topic but not the source, so that reviewers know the change was written
under these rules.
