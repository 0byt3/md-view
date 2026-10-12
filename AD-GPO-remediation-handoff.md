# AD / Group Policy remediation: open items

Domain `example.com` (2016 DFL, PDC `DC02`, DCs `DC01`). Compiled 2026-09-02 from a read-only review of all 58 GPOs, SYSVOL, security filtering, and the live script share. Items already completed by Ben are listed at the end so they aren't redone.

Priority key: **P1** = domain-compromise path or plaintext secret, do first. **P2** = credential-theft / lateral-movement hardening. **P3** = hygiene and structure.

| #   | Item                                                                   | Priority | Effort                  | Complete? | Notes                                                                                                                                 |
| --- | ---------------------------------------------------------------------- | -------- | ----------------------- | --------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Lock down `\\example.com\common\it` write access                       | P1       | 1 h                     |           | Previously Domain Admin write-only was restricted to the common\\it\\scripts directory. Restricting edit to common\\it is better.     |
| 2   | Move GPO scripts to NETLOGON (script provided)                         | P1       | 2 h + soak              | y         | All scripts copied to netlogon and GPOs changed to source from netlogon                                                               |
| 3   | Rotate App Automate `SIMPLEPASS`                                       | P1       | 30 min                  |           | Lets go.                                                                                                                              |
| 4   | Plaintext secrets still on the share                                   | P1       | half day                |           | I didn't realize there was a script in SYSVOL\\Policies as the previous reference was to ..\\common\\it before I moved it to netlogon |
| 5   | Re-enable Point and Print admin-only driver install                    | P2       | 1 h + pilot             |           | I really like this, but will need to test that pre-loading drivers works for when adding the printer                                  |
| 6   | WinRM / RDP / firewall scoping and CredSSP off                         | P2       | half day                |           | CredSSP changed to off. RDP and SSH blocking was done on UniFi, updated to include WinRM. Can discuss Windows Firewall, may be useful |
| 7   | Tier privileged accounts (deny DA on workstations, LAPS for elevation) | P2       | half day                |           | This is a great idea. Doesn't just have to be a guideline, but also GPO enforced.                                                     |
| 8   | Protected Users for human admin accounts                               | P2       | 1 h + test              |           |                                                                                                                                       |
| 10  | Banned-password filter (Entra PP or Lithnet)                           | P2       | half day + audit period |           |                                                                                                                                       |
| 11  | Block Inheritance on Domain Controllers / Domain Servers OUs           | P2       | 1 h verify              | y         |                                                                                                                                       |
| 12  | `StrongCertificateBindingEnforcement` GPP item                         | P3       | 30 min                  |           |                                                                                                                                       |
| 13  | Fine-Grained Password Policy for admins/service accounts               | P3       | 30 min                  |           |                                                                                                                                       |
| 14  | SYSVOL and NETLOGON cleanup                                            | P3       | 1 h                     |           | Just have to confirm with Matt regarding this                                                                                         |
| 15  | GPO clutter, filtering, links, stale objects                           | P3       | ongoing                 |           |                                                                                                                                       |
| 16  | GPO structure / naming consolidation                                   | P3       | project                 |           |                                                                                                                                       |

Take `Backup-GPO -All -Path <path>` (Windows PowerShell 5.1, not pwsh) before any of the GPO edits below.

______________________________________________________________________

## 2. Move GPO scripts into NETLOGON (P1)

**Finding.** 40 `psscripts.ini`/`scripts.ini` entries across 15 GPOs point at `\\example.com\common\it\scripts\{startup,logon}`. The SYSVOL copies of those scripts are stale mirrors that nothing references.

**Action.** Run `C:\Users\user1\gpo-tools\Move-GpoScriptsToNetlogon.ps1` as a Domain Admin in **Windows PowerShell 5.1**:

```powershell
.\Move-GpoScriptsToNetlogon.ps1 -BackupPath D:\gpo-backups           # dry run, prints every change
.\Move-GpoScriptsToNetlogon.ps1 -BackupPath D:\gpo-backups -Commit   # apply
```

What it does: `Backup-GPO` on every touched GPO; mirrors `startup\` and `logon\` into `\\EMIL-DC02\NETLOGON\startup|logon`; copies `tar.exe`, `archiveint.dll`, `printer-drivers.csv` into `NETLOGON\lib`; rewrites hard-coded share paths inside the copied scripts (`Install-App10.ps1`, `Install-PrinterDrivers.ps1`, `Configure-App10.ps1`); rewrites the INI entries to `\\example.com\NETLOGON\...`; bumps the machine/user version number of each GPO in AD and `GPT.ini` so clients re-read the script list. It deletes nothing from the share. Dry run has been validated read-only against the live domain.

Known exceptions it reports:

- `Test - Software - App8 Upgrade` references `Install-App8.ps1`, which does not exist. Fix or unlink in GPMC.

**Verify.** After DFSR converges (`repadmin /syncall EMIL-DC02 /AdeP`; `dfsrdiag pollad`), on a test workstation: `gpupdate /force`, reboot, check `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Group Policy\Scripts\Startup\*` shows NETLOGON paths and the Group Policy operational log has no script errors. After one to two weeks, make the share read-only (item 1) and delete `scripts\startup` and `scripts\logon`.

**Ongoing workflow.** Keep scripts in a git repo; deploy with `robocopy <repo>\startup \\EMIL-DC02\NETLOGON\startup /MIR` (same for `logon`, `lib`). Always write to the PDC. GPOs only need touching when a script is added or removed. Optional hardening: sign the scripts and set *Turn on Script Execution: Allow only signed scripts* for the machine policy.

## 3. Rotate the ConnectWise Automate agent password (P1)

**Finding.** The SYSVOL copy of `Install-ConnectWise.ps1` (Default Domain Policy, dated 2024-01) contained the RMM `SERVERPASS` in plaintext and was readable by every authenticated user until Ben deleted it on 2026-09-02. The live share copy was cleaned in or before May 2026.

**Action.** Rotate the Automate agent install password in the Automate console; update the live `Install-ConnectWise.ps1` so it does not embed the new value (fetch from a Domain-Admins-only location, or use the Automate deployment token mechanism). Confirm no other copies exist: `Select-String -Path \\example.com\common\it\scripts\*.ps1 -Recurse -Pattern SERVERPASS`.

## 5. Point and Print: re-enable admin-only driver installation (P2)

**Finding.** Default Domain Policy has *Limits print driver installation to Administrators* explicitly **Disabled** (`RestrictDriverInstallationToAdministrators = 0`), plus `NoWarningNoElevationOnInstall = 1` and `UpdatePromptSettings = 2`. This is the PrintNightmare-vulnerable configuration; the only mitigation is the approved-servers list (`kel-fs1`, `ter-fs1` after Ben removed `emil-dc02`), so anyone who compromises or spoofs those servers gets SYSTEM on every workstation with no prompt.

**Why it is safe to change.** The restriction applies only to Point and Print (`\\server\share` connections) when the driver is not already present. `Install-PrinterDrivers.ps1` already pre-stages the KM/Ricoh/Canon/Lexmark/Brother universal drivers as SYSTEM from `printer-drivers.csv`, and the GPP-deployed shared printers are Type 4 / class-driver connections. Home printers are unaffected: those go through WSD/IPP/PnP (Microsoft IPP Class Driver or Windows Update drivers) and standard users can add them today and after the change. The only new friction is a brand-new driver family, which is an IT event anyway (add a line to `printer-drivers.csv`).

**Action.**

1. Confirm driver search on Windows Update is not disabled anywhere (*Specify search order for device driver source locations* / `SearchOrderConfig`).
1. Pilot: small GPO or test OU with the setting **Enabled** (or Not Configured; post-KB5005652 default is Enabled). Include one laptop that goes home. Watch `Microsoft-Windows-PrintService/Admin` for events 215/808.
1. Roll into the DDP (or the future workstation baseline). Add missing drivers to `printer-drivers.csv` rather than loosening the policy.
1. Longer term: evaluate Windows Protected Print mode (Win11 24H2+); removes vendor drivers entirely but also copier features like account-tracking codes.

Path: DDP → Computer Configuration → Policies → Administrative Templates → Printers.

## 6. WinRM, RDP, firewall scoping; CredSSP off (P2)

**Finding.** DDP auto-configures a WinRM listener on all domain machines with `IPv4Filter = 10.0.0.0/8`; CredSSP is enabled on both WinRM client and service, with *Allow delegating fresh credentials* to `WSMAN/*`. Workstations allow RDP, Remote Administration, and File & Print inbound from `10.0.0.0/8` and `192.168.0.0/16`; `Servers` allows RDP and Remote Admin from any address. Net effect: every workstation is a WinRM/RDP target from every other workstation, and any admin connecting to a compromised workstation with CredSSP hands over reusable credentials.

**Action.** Keep WinRM (Kerberos PowerShell remoting is the one remote method that leaves nothing reusable on the target), but scope it:

- WinRM Service → *Allow remote server management through WinRM*: IPv4 filter = admin subnet(s) / jump host / VPN admin pool instead of `10.0.0.0/8`. Also scope the *Windows Remote Management (HTTP-In)* firewall rule's remote addresses the same way; both gates are independent.
- WinRM Service → *Allow CredSSP authentication*: **Disabled** on Workstations and Servers. WinRM Client → *Allow CredSSP*: Disabled; remove the `WSMAN/*` fresh-credential delegation. If one double-hop is genuinely needed, delegate to that single SPN only, or use `Copy-Item -ToSession` / `Invoke-Command -Credential` / resource-based constrained delegation.
- Scope the RDP, Remote Admin, and File & Print inbound rules on workstations to the same admin sources. Fix `Servers` RDP/Remote Admin "any address".
- The historical remote-management pain (perfmon, wf.msc) was RPC/DCOM, not WinRM: enable the specific rule groups (*Performance Logs and Alerts*, *Windows Firewall Remote Management*, *Remote Event Log Management*, *Remote Service Management*, *WMI*) scoped to the admin subnet, and set Remote Registry to Manual (trigger start). Or do the same tasks over WinRM (`Invoke-Command { Get-Counter }`, `Get-NetFirewallRule -CimSession`).

## 7. Privileged-account tiering on workstations (P2)

**Finding.** Domain Admin credentials are routinely typed into workstations (UAC prompts over ConnectWise sessions, RDP). Any compromised endpoint then holds a DA credential in LSASS.

**Action.**

- `Workstations` GPO → User Rights Assignment: *Deny log on locally*, *Deny log on through Remote Desktop Services*, *Deny log on as a batch job*, *Deny log on as a service* for `Domain Admins`. Do **not** deny *Access this computer from the network* (breaks remote management). This makes typing a DA password into a workstation fail, turning the habit into a rule.
- Elevate on workstations with the LAPS local admin (`Get-LapsADPassword -Identity <PC> -AsPlainText`), falling back to the shared `wsadmin` workstation-tier account. Use ConnectWise Backstage / Automate command shell (already SYSTEM) where no credential is needed at all.
- RDP to workstations: enable *Restricted Admin* or *Remote Credential Guard* so RDP leaves no reusable credential.
- Defence in depth on endpoints: LSA Protection (`RunAsPPL`) and Credential Guard (supported on Windows 11 Pro/Business 22H2+; verify on a pilot before fleet-wide).
- UAC: `PromptOnSecureDesktop = 0` in the DDP should return to 1.

## 8. Protected Users (P2)

**Finding.** Group is empty.

**Action.** Add the human admin accounts (`admin.user1`, `admin.user2`, `admin.user3`) one at a time, starting with one and testing RDP to a DC, `Get-ADUser` from a workstation, and GPMC. Do not add regular users (no cached logon means no sign-in when a DC is unreachable), service accounts, computer accounts, `wsadmin`, or the built-in `Administrator` until a break-glass path is tested. Members lose NTLM (no connecting by IP), DES/RC4, delegation, and cached credentials; TGT is 4 h non-renewable.

## 10. Banned-password filter (P2)

**Finding.** Domain policy is 8 chars, complexity on, 24 history, no expiry, lockout 10/15 min. Nothing blocks `Summer2026!`, company/site names, or leetspeak variants. Neither Entra Password Protection nor Lithnet appears to be installed (no GPO references; DC service list could not be read from a standard account, so confirm).

**Decision needed.** Both are password-filter DLLs on the DCs; neither can be scoped per user.

- **Entra Password Protection for on-prem AD** (preferred if licensing is acceptable). Global banned list plus a custom list of up to 1,000 terms with fuzzy/leet normalisation. Licensing: Entra ID P1 for every synced user; included in M365 Business Premium, honour-system for the users without it (no technical gate). Big UX advantage for this environment: SSPR/myaccount password changes are evaluated in the cloud against the same lists **before** writeback and give a specific, friendly rejection message. Setup: proxy service on a member server with outbound HTTPS (two for redundancy), DC agent on all three DCs (reboot each), Entra portal → Protection → Authentication methods → Password protection: custom list, *Enable on Windows Server AD* = Yes, Mode = **Audit** for two weeks, review `Microsoft-AzureADPasswordProtection-DCAgent/Admin` (events 10025/30009), then Enforce.
- **Lithnet Password Protection for AD** (free, open source). Banned words with normalisation, regex, optional HIBP compromised-hash store (~10 GB per DC). No licensing question. Downside: cloud SSPR knows nothing of the rules, so a rejected password fails at writeback with a generic error.
- Either way, Ctrl+Alt+Del changes show only the stock "does not meet the password policy requirements" message; put a one-paragraph explanation in user guidance.

## 11. Block Inheritance on Domain Controllers and Domain Servers OUs (P2, verify)

**Finding.** Both OUs block inheritance and the DDP is not Enforced. Microsoft guidance is never to do this on the DC OU. Consequences to verify: DDP account-policy edits may no longer propagate to the PDC (current effective policy matches the DDP, probably written before the block); the two enterprise root CA certs, NTP, BitLocker, audit, WinRM, firewall and certificate autoenrollment settings delivered via the DDP do not reach DCs or servers in Domain Servers.

**Action.** `gpresult /h` on a DC and on a server in Domain Servers; compare against intent. Either remove the block (preferred; move DC-specific overrides into the Default Domain Controllers Policy, which has higher precedence anyway) or Enforce the DDP and accept that everything in it lands on DCs. Shrinking the DDP to account/Kerberos policy only (item 16) makes this decision much easier.

## 12. `StrongCertificateBindingEnforcement` GPP item (P3)

**Finding.** Default Domain Controllers Policy → Computer Configuration → Preferences → Windows Settings → Registry → item `StrongCertificateBindingEnforcement` writes `HKLM\SYSTEM\CurrentControlSet\Services\Kdc\StrongCertificateBindingEnforcement = 1` (Compatibility mode). KB5014754: default became Full Enforcement in Feb 2025 and the key is ignored on DCs patched after 2025-09-09. So it is dead configuration on patched DCs and a weakening on any DC that is behind. No Kdcsvc 39/40/41 events on EMIL-DC02 in the last 30 days, so no weak certificate mappings are in use.

**Action.** Confirm all three DCs have the September 2025 (or later) cumulative update. Change the GPP item action to **Delete**, let each DC run `gpupdate`, verify with `Get-ItemProperty HKLM:\SYSTEM\CurrentControlSet\Services\Kdc`, then remove the item.

## 13. Fine-Grained Password Policy (P3)

Ten-minute safeguard against future mistakes (helpdesk reset to `Welcome1!` on a DA, vendor-set service account password) and an insurance-questionnaire checkbox. Group-based, so nothing to remember once applied. ADAC → System → Password Settings Container → New PSO: `Privileged Accounts`, min length 15, history 24, max age 180 d, lockout 5 / 30 min, applied to `Domain Admins` (and `Enterprise Admins`). Optionally a second PSO for a `Service Accounts` group: 25+ chars, never expires.

## 14. SYSVOL and NETLOGON cleanup (P3)

Do on the PDC path (`\\EMIL-DC02\SYSVOL\example.com\Policies\...`); DFSR replicates deletions. `Backup-GPO -All` first.

- **True orphan folders** (AD object gone, confirmed with `Get-GPO -Guid`): `{1219D80B-E2DE-4B28-A676-0BA9DE7A9BFA}`, `{26332BF3-63A1-4DD4-836C-9FD4AB7EE6D3}`, `{9867A221-9DC6-4E8F-9E7D-3686079B67AE}`. Delete.
- **Restricted, not orphaned**: `{2F1405B2-BC24-4E1E-A451-CF3FAF407A54}` (linked at `RoadsAutoLogin`), `{D90AACAE-950E-4D8D-A151-2E1B213633F9}` (linked at the Users OU). Authenticated Users read was removed; likely intentional for the autologon GPO. Confirm in GPMC as an admin, leave alone.
- **24 stale script copies** in GPO `Scripts\Startup|Logon|Logoff` folders that no INI references (5 in DDP, 10 in `Workstations`, 3 in `Redirected Folders - OneDrive Handover`, 2 in `Software - App4`, 1 each in `Software - App5`, `Software - App6`, `RDS`). Safe to delete; does not change GPO versions. Becomes entirely moot after item 2 if you prefer to delete them then.
- **NETLOGON root** has 8 stale files (2022–2026: `Clean-Printers.ps1`, `Enable-BitLockerVolumes.ps1`, `Install-SharedPrinterDrivers.ps1`, `Remove-App0.ps1`, `Remove-App7.ps1`. No GPO references them. Delete after item 2 lands (the new layout uses `startup\`, `logon\`, `lib\` subfolders, so there is no collision).
- `PolicyDefinitions` (117 MB) is the central ADMX store; leave it.

## 15. GPO clutter, filtering, links, stale objects (P3)

- **Broken reference**: `Test - Software - App1 Upgrade` → `Install-App1.ps1` does not exist.
- **Unlinked GPOs (9)** including four `Undo` GPOs and four WPAD variants (two disable, one enable, one undo). Consolidate to one and delete the rest once the rollback window has passed.
- **`Test -` GPOs (10)**, some from 2022–2023. `Test - WHfB` is linked at the domain root; `Test - Recycle Bin lifetime` and `Test - App2 Install` have empty security filtering and do nothing. Graduate or delete.
- **Filtering by individual principals**: `Software - TAD` (38 named users plus one orphaned SID), `Settings - Disable Office Hardware Acceleration` (7 users), `Test - Classic Outlook Install` (11 named computers), `Redirected Folders` (2 users allowed, 97 denied; looks fully migrated and retirable). Convert to groups.
- **OU placement**: 6 servers and 3 Linux boxes sit in the flat `Computers-Managed` OU with 463 workstations; the `Servers` GPO is linked there with a WMI filter as a fallback. Move them to the server OU so the WMI filter is a safety net, not the mechanism.
- **Stale accounts**: 144 enabled workstation accounts and 5 servers with no logon in 90+ days. Disable/move per your lifecycle policy.
- **Unsupported builds**: ~10 machines still on 22000/22621.
- **Cosmetic**: `Servers` WMI filter description says `ProductType = 1`; `sharepoint.microsft.com` typo in the Servers Edge allow-list; `tdcommericalbanking` GPO name.
- Legacy LAPS → Windows LAPS migration when convenient.

## 16. GPO structure and naming (project) (P3)

Current state: 30 GPOs linked to `Computers-Managed`, targeting done almost entirely by WMI filters and security groups on a flat OU; the DDP has become a kitchen sink (software installs, printers, drives, ODBC DSNs, Edge favourites, Office policy, firewall, WinRM, BitLocker, cert autoenrollment, WHfB). Recommended target, one GPO = one change-management unit (same population, same owner, staged and rolled back together):

- **Baselines per device class** (`WS-Baseline`, `SRV-Baseline`, `DC-Baseline`): fold in the fully rolled-out single-setting GPOs (`Security - Disable SMBv1`, `Require SMB Signing`, `Disable WPAD`, LLMNR/NetBIOS, spooler off on DCs).
- **DDP** shrinks to account policy and Kerberos policy only (this also defuses item 11).
- **Concern-scoped GPOs** with their own lifecycle: `ALL-Security-NTLM`, `WS-Config-Print`, `WS-Config-Browser`, `WS-Config-WindowsUpdate`, `WS-Config-OneDrive`, `WS-Config-BitLocker`, `ALL-Config-RemoteManagement`.
- **One GPO per app** filtered by an `App - X` group (already the practice; keep).
- **Role overlays** (`Kiosks`, `RDS`, `Hyper-V Hosts`) stay small.
- `Test -` and `Undo` GPOs are time-boxed: graduate into a baseline or delete; never linked at the domain root for years.
- Target roughly 12–18 linked GPOs on the workstation OU, down from 30. Use `Get-GPOReport -All -ReportType Html` to review before consolidating.
- Longer term, much of the workstation layer is an Intune candidate (devices already Entra-registered, Intune test GPO exists), which would turn this into a migration rather than a refactor.

______________________________________________________________________

## Already done (2026-09-02, Ben)

- Deleted the SYSVOL copy of `Install-App3.ps1` containing the plaintext `SIMPLEPASS` (rotation still pending, item 3).
- Removed `dc02` from both Point and Print server lists in the DDP.
- Confirmed RSAT Group Policy Management Tools are installed; the `GroupPolicy` module is Desktop-edition only, so use `powershell.exe` 5.1 or `Import-Module GroupPolicy -UseWindowsPowerShell` in pwsh.
- Migration script `Move-GpoScriptsToNetlogon.ps1` written and dry-run validated (item 2).

## What is already good (for context)

SMB signing required, SMBv1 removed, LDAP signing and channel binding, outbound NTLM blocked on DCs, Kerberos AES-only, LLMNR/NetBIOS/WPAD off, BitLocker with AD key escrow, LAPS, Sysmon plus Wazuh, advanced audit policy, Windows Update deadlines, OneDrive KFM, unused GPO halves disabled, descriptive `desc` fields on GPP items. The 2026 security work is deliberate and the items above build on it rather than replace it.
