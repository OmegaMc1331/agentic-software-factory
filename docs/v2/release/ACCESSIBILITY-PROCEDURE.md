# Native accessibility and input qualification

Automated engine snapshots, keyboard navigation and contrast findings are retained
separately from native assistive technology. No universal WCAG conformance or native
screen-reader support is asserted from Axe alone.

On each actual supported desktop, record OS/browser/AT versions and the exact
candidate source/binary hashes. Use a disposable project and no production secrets.
Verify spoken or AT-generated output, not merely a screenshot of a focused control.

1. Launch the packaged V2 application and navigate the project selector by keyboard.
2. With NVDA/Narrator on Windows or VoiceOver on macOS, inspect the activity rail,
   project identity, workspace tabs and active panel announcements.
3. Open a file, edit/save, leave CodeMirror with its documented Tab-focus mode, and
   verify the save/conflict announcement without losing the buffer.
4. Open/close the command palette; search, choose a command, verify focus restoration.
5. Open Settings; search a real setting, change/save it and verify acknowledged state.
6. Open a genuine notification and navigate to its owning mission; verify read state.
7. Inspect review/diff controls, comments, tree navigation and accessible docking
   alternatives. Confirm terminal shortcuts remain available when it has focus.
8. Exercise dialogs, errors, denied actions and reduced motion in both themes.
9. With an installed Japanese/Chinese IME, compose in conversation and CodeMirror.
   Enter must confirm composition rather than submit a message or trigger a global
   command. Also test accented Latin text and actual Unicode file paths.
10. Record task completion, exact output, focus defects, severe blockers and limits.

Windows Narrator is detected on the qualification host; NVDA is not detected in
PATH. Detection is not execution qualification. Synthetic composing events can
verify the application's `isComposing` guard but not a physical IME journey.
Browser-engine keyboard automation likewise does not certify native speech/audio
usability. Missing native/human observations remain UNVERIFIED, not PASS.
