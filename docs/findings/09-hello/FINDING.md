# Finding: Spike P6, Windows Hello above the overlay

Status: blank. The checklist has not been run.

Part of issue 09. Do not treat either decision below as chosen.

## Decision

Leave both boxes empty until the checklist is finished.

- [ ] Go. Hello face or fingerprint, Hello PIN through passthrough, and the offline network logon are good enough to build tickets 21 and 22 on.
- [ ] Fallback. Password plus the SoftLock PIN only, and Hello face or fingerprint only (no Hello PIN passthrough).

Checked by:

Date:

## Dependents

- Ticket 21: Unlock with the Windows account password, with backoff. It needs the offline network-logon result.
- Ticket 22: Unlock with Windows Hello through the Broker. It needs the dialog, passthrough, and chord results.

## Results

Write what happened. Leave a line as `(blank)` if that run was not done.

### Hello dialog above the overlay

(blank)

### Face

(blank)

### Fingerprint

(blank)

### PIN through passthrough

(blank)

### Win during passthrough

(blank)

### Alt+Tab during passthrough

(blank)

### Ctrl+Shift+Esc during passthrough

(blank)

### Passthrough ended when the dialog closed

(blank)

### Key reached nothing after the dialog closed

(blank)

### Passthrough ended after 60 seconds

(blank)

### Key reached nothing after 60 seconds

(blank)

### Password offline, Microsoft account

Account kind: (blank)

Result: (blank)

Win32 error: (blank)

### Password offline, local account

Account kind: (blank)

Result: (blank)

Win32 error: (blank)

### Account lockout

How many wrong passwords were tried: (blank)

Did Windows lock the account: (blank)

### Foreground path printed by the tool

(blank)

### Notes for tickets 21 and 22

(blank)
