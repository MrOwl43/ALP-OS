# ALP-OS Live: experience plan, the first journey

**2026-10-19 · Draft for discussion.** Based on the manifesto, spec v0.5, the decisions log, the 2026-10-18 reviews, the three screenshots and the Stream A technical thread up to that night's decisions (external disk only, an encrypted index on the USB stick, an admin password the user chooses). No code was touched.

---

## 1. How the product should feel

| Feeling | What it means in practice |
|---|---|
| **"My photos are at home"** | Every screen talks about photos, phones and safety. Never about partitions, services or certificates. |
| **Calm, not technical** | One question per screen. A good default is always selected. No screen is frightening. |
| **Honest and trustworthy** | Say plainly what is protected and what is not ("If the USB stick is lost..."). Never promise what is not built. |
| **One moment of magic** | The moment photos from the phone start appearing on the big screen. The whole journey is built to get there fast. |
| **Quiet after setup** | Once everything works, the system fades into the background. It speaks only when something needs attention. |

## 2. The first journey, step by step

Status: ✓ exists in the screens · ◐ partly there or out of date · ○ missing

| # | Step | What the user does and feels | Status | Gap or note |
|---|---|---|---|---|
| 0 | **Preparation** | Downloads a file, writes a USB stick, changes BIOS settings (Secure Boot off, boot menu) | ◐ text guide | **The biggest drop-off point of the journey.** For a non-technical user this is a wall. Fine for the alpha, but it defines who can be a tester. |
| 1 | **Start-up** | A dark screen for up to a minute | ◐ | A splash screen with the logo instead of a black screen (idea from PR #7, in progress). |
| 2 | **Welcome and hardware check** | Picks a language, sees "This computer is suitable" | ✓ | |
| 3 | **Network** | Picks a Wi-Fi network and types the password | ✓ | Looks good. |
| 4 | **Admin password** | Chooses a password | ○ | New from that night. Needs a screen, plus a one-line explanation: why it is needed and when it will be asked. |
| 5 | **Where the photos are kept** | Plugs in an external disk and picks it | ◐ out of date | The current screen offers the Windows partition (the status says "Kept in Windows / ALPArchive"). Needed: "Plug in an external disk", automatic detection, a gentle explanation of why the Windows disk is not offered, and how many photos will fit. |
| 6 | **Connecting the phone** | Installs Immich, scans a QR code, types a user name and password | ◐ | **The highest friction after the BIOS.** Typing a password like `kite-river-amber-58` on a phone, and the worry that the home security certificate will be blocked in the app (my inference, to check against Immich). The spec says the user types no address, account or password. |
| 7 | **Sync chain** | ? | ◐ | Unclear what the user does here in the alpha and what it gives them. From their point of view the phone is already connected. |
| 8 | **All set** | Sees "ALPArchive is working" | ✓ | Needs updating to "Kept on the external disk". |
| 9 | **The first photo arrives** | Watches the photos arrive | ◐ | Today it is a line of text ("148 photos kept so far"). This is the moment most worth investing in: thumbnails that come in live. |
| 10 | **The next start** | Types the admin password | ○ | **The encrypted index password means that after a power cut the backup does not come back until someone types a password on the screen.** This clashes with "the box comes back by itself" in the spec. |
| 11 | **Rewriting the USB stick** | Writes a new version | ○ | The key is erased and the archive is lost. Decided: a warning for now. Worth considering a recovery page or a QR code to photograph with the phone. |

**Proposed structure:** 7 progress dots instead of 6. Welcome → network → password → disk → phone → (photos arrive) → all set. The chain merges into the phone step, because for the user connecting the phone *is* joining the chain.

## 3. The three moments that matter most

1. **"It works on my computer"** (step 2): the first thing the user sees is a clear, warm answer, not a parts list.
2. **"My photos are here"** (step 9): photos from the phone appear on the big screen within seconds of the scan. This is what a user will tell friends about.
3. **"I can trust this"** (steps 10 and 11): what happens after a power cut, after a shutdown, and if something is lost. This is where trust is built or broken.

## 4. After setup: what is on the screen

Today the status screen is cards with numbers. There are at least two approaches, and the choice defines the character of the product: a **status board** (as now, quiet and informative) or a **photo frame** (recent photos rotate, with status in a small bar). A photo frame delivers "a family album, not a computer", but it shows photos to whoever stands in front of the screen. So it is possible only after unlocking with the password.

## 5. Open questions, by priority

Each question was asked separately, one after another.

| # | Question | My recommendation |
|---|---|---|
| 1 | **Who tests the alpha?** ✅ **Decided (2026-10-19): friends with basic skills.** A clear guide for writing the stick and the BIOS, and the screens must work without help. | |
| 2 | **What happens after a power cut?** ✅ **Decided (2026-10-19): automatic unlock.** With a TPM chip the index opens by itself and the backup resumes. Without a chip, the admin password is typed. | |
| 3 | **How does the phone connect?** ✅ **Decided (2026-10-19): a short password.** A QR code for the address, and a short, readable password typed once. A QR code that fills in everything in the beta. | |
| 4 | **What is on the screen day to day?** ✅ **Decided (2026-10-19): a locked status board.** A lock screen like Windows, showing only non-sensitive data: an icon for each synced device with a masked name, network connection, free space, disk health and so on. To enter the graphical UI (see photos, scroll and search) you pass a sign-in screen with a password. Inside the UI, deleting a file and emptying the trash ask for the password again. In Settings a photo-frame screensaver can be turned on (off by default). The principle: "The computer stands in the living room, and only those who need to know how to use it. A curious child cannot get in and make a mess." | |
| 5 | **The sync chain in the alpha:** ✅ **Decided (2026-10-19): merged with connecting the phone.** A phone that connects joins the chain behind the scenes. The seed appears in Settings when it is needed. | |
| 6 | **Language for the alpha:** ✅ **Decided (2026-10-19): English only**, as decided before. The right-to-left groundwork stays. | |
| 7 | **Rewriting the stick erases the key:** ✅ **Decided (2026-10-19): a recovery page with a QR code.** A screen at the end of setup shows a recovery key and a QR code to photograph or print. With the key the backup can be opened on a new USB stick. | |
| 8 | **The phone app is called Immich:** ✅ **Decided (2026-10-19): use the Immich app, rebrand later.** The phone-connection screen shows an asterisk with a short note. Proposed wording: "\* For now ALPArchive uses Immich, a trusted open-source photo app. Our own ALPArchive app is on the way." | |
| 9 | **The product's voice:** ✅ **Decided (2026-10-19): warm and short.** Talk about photos and home, one sentence for each thing ("Your photos are home and safe"). | |

## 6. Summary of the 2026-10-19 decisions and the updated journey

**Setup path (6 dots):** welcome and hardware check → network → admin password → external disk → connecting the phone (including joining the chain, and the first photos arrive) → recovery page with a QR code → all set.

**Day to day:** a lock screen that shows non-sensitive status. A password once to enter the gallery and search, and again only to delete and to empty the trash. After a power cut everything comes back by itself with a TPM. A photo frame is an optional screensaver.

**Visual language (later, after the alpha):** icons and short animations that illustrate each concept, especially where there is a technical term. An idea from the owner: show the disk usage as a pizza. Each slice is a type of content (photos, videos, index), the missing part is the free space, and a new device adds a "topping". That way "out of space" looks like a pizza that is almost finished, not a threatening chart. **Next to the pizza there is always a serious bar with all the numbers** (gigabytes, percentages, how many more photos will fit). The principle: show it in a friendly way, but also in a serious way, always both.

**All decisions were passed to the Stream A technical thread.**
