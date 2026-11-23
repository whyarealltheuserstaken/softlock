# Ctrl+Alt+Del and UAC while locked

Spike 03 did not fill its screen tables. The gaps below are the ones the spec already says policy cannot close, plus `NoClose`, whose effect on the secure-desktop power icon was not measured. Photos and the per-edition yes/no table stay on Henry's checklist (issue 17). This note is what the user guide (issue 30) should say.

The product name in the program is the name constant. This page uses that working name, SoftLock.

## While locked

SoftLock writes these values only after the settings journal has been saved. The journal stores the previous value, including a value that was absent, and the locked user's SID. User values are written under that SID. Machine values are written for the computer. A Group Policy overwrite is written back within 5 seconds, and sooner when the policy key changes.

| What you press or see | While SoftLock is locked | After unlock | After a crash or power loss |
| --- | --- | --- | --- |
| Task Manager | Removed. Ctrl+Shift+Esc does not open it. | The previous value is back. | The previous value is back when the service starts and the Lock Host is gone. If the Lock Host is still running in an active session, the value stays and SoftLock fails closed. It does not return to Unlocked. |
| Sign out (`NoLogoff`) | Removed from Ctrl+Alt+Del. | Previous value restored. | Same restore rule as Task Manager. |
| Change a password (`DisableChangePassword`) | Removed. | Previous value restored, including an administrator's 1. | Same restore rule. |
| Switch user (`HideFastUserSwitching`) | Removed. This one is a machine value. | Previous machine value restored. | Same restore rule. The SID in the journal is still the user who was locked. |
| Lock (`DisableLockWorkstation`) | Removed. A physical Win+L does nothing. The service deletes this one value immediately before `LockWorkStation`, after the journal records the clear, and sets it back to 1 as soon as that attempt finishes, whether the real lock was confirmed or the fall-back was a disconnect. If the service is already gone, the Lock Host cannot update the journal, so it disconnects its own session and does not call `LockWorkStation`. The disconnect does not delete the value. | Previous value restored. | The journal keeps the previous value, not the temporary clear, so a crash does not leave the value deleted. |
| Shut down (`NoClose`) | Applied. It removes shutdown from the signed-in session. It may not hide the power icon on the secure desktop. That was not measured. | Previous value restored. | Same restore rule. |
| UAC for an administrator (`ConsentPromptBehaviorAdmin`) | The prompt asks for credentials. A click on Yes is not enough. | The previous prompt behavior is restored, including an administrator value such as 5. | Same restore rule. |
| UAC for a standard user (`ConsentPromptBehaviorUser`) | The prompt is denied on its own. There is nothing to click. | The previous prompt behavior is restored, including 1 or 3. | Same restore rule. |

Cancel on the Ctrl+Alt+Del screen returns to the desktop, where SoftLock is still showing. SoftLock does not swallow Ctrl+Alt+Del.

## Signing back in

Windows Hello and the account password at the real Windows lock screen keep working. SoftLock does not change sign-in policy, credential providers, or smart-card enforcement. The UAC values above apply to elevation prompts, not to the lock screen. Clearing `DisableLockWorkstation` is what lets the real lock appear. Restoring it afterward does not sign the owner out and does not block the password or Hello prompt that is already up.

## What policy does not close

These stay available. They are limitations, not a second lock.

- The power icon may still show. Restarting the computer ends at the Windows sign-in screen. That is an accepted way out.
- The network icon may still show. It does not open the desktop.
- The accessibility icon may still show. What it can start (Narrator, the on-screen keyboard, Magnifier) is ticket 19, not this one.
- Keyboard Filter, which can suppress Ctrl+Alt+Del on Enterprise and Education editions, is deferred.

Henry still has to look at the real screen on Windows 10 and Windows 11, on the laptop and on the school workstation, and record which icons remain.
