// Letter template library.
//
// A curated set of Letter prose starters. The frontend uses these to prefill
// the compose form. They are intentionally written in a careful, plain voice
// that matches the brand — never the chatbot voice.
//
// The `kind` and `category` fields drive the server-side classification (see
// migrations/0006_letter_categories.sql). The body is a markdown-ish prose
// template with `«PLACEHOLDER»` markers the Principal replaces.

export type LetterKind =
  | 'MESSAGE'
  | 'CREDENTIAL_BUNDLE'
  | 'FILE_ARCHIVE'
  | 'ACTION'
  | 'WILL_LOCATOR'
  | 'VIDEO_MESSAGE'
  | 'AUDIO_MESSAGE'
  | 'MEDIA_PLAYLIST';

export type ReleaseMode = 'SIGNAL_OR_SCHEDULED' | 'SCHEDULED_ONLY' | 'SIGNAL_ONLY';

export interface LetterTemplate {
  /** Stable identifier — used as the server-side `category`. */
  id: string;
  /** Display label in the picker. */
  name: string;
  /** Server-side `kind`. */
  kind: LetterKind;
  /** Two-line description shown next to the name. */
  description: string;
  /** Suggested title for the Letter. */
  title: string;
  /** Recommended retention class — Draft means single-Letter free tier is OK. */
  recommendedFor?: 'Draft' | 'Estate' | 'Estate+' | 'Legacy';
  /** Suggested release mode. Time-capsule templates use SCHEDULED_ONLY. */
  releaseMode?: ReleaseMode;
  /** Prose template the Principal edits. */
  body: string;
}

export const LETTER_TEMPLATES: LetterTemplate[] = [
  {
    id: 'will_locator',
    name: 'Where my will is',
    kind: 'WILL_LOCATOR',
    description:
      'A structured "if anything happens, here is the legal will" letter. Does NOT replace a will — points to one.',
    title: 'Where my will lives',
    recommendedFor: 'Estate',
    body: `My current Last Will and Testament is dated «DATE EXECUTED».

The signed original is held at:
  «PHYSICAL LOCATION — e.g. safe deposit box at WestpacBranch, top drawer of writing desk»

A scanned copy is at:
  «DIGITAL LOCATION — e.g. iCloud Drive › Personal › Will-2026.pdf»

My estate solicitor is:
  Name:       «FULL NAME»
  Firm:       «FIRM NAME»
  Phone:      «PHONE»
  Email:      «EMAIL»

My executor(s) (already named in the will):
  1. «NAME» — «PHONE / EMAIL»
  2. «NAME» — «PHONE / EMAIL»

The will was witnessed by:
  1. «NAME» — «CONTACT»
  2. «NAME» — «CONTACT»

If you cannot reach my solicitor, the executor(s) can probate the will directly via «JURISDICTION» courts.

Important: anything in this letter is for orientation only. The will itself is the binding document. If anything in this letter contradicts the will, the will wins.`
  },

  {
    id: 'letter_to_children',
    name: 'Letter to my children',
    kind: 'MESSAGE',
    description: 'For the things you want to say that have nothing to do with paperwork.',
    title: 'For «CHILD\'S NAME»',
    recommendedFor: 'Estate',
    body: `«CHILD'S NAME»,

If you're reading this, the people at Paschal decided it was time. That means I'm not there to say this in person, and I'm sorry about that.

A few things I want you to know.

«WHAT YOU WANT THEM TO KNOW. There is no right shape for this. You can write a paragraph or twelve pages. The system will hold whatever you write, exactly.»

I love you.

— «YOUR NAME / "Mum" / "Dad"»`
  },

  {
    id: 'funeral_preferences',
    name: 'Funeral & memorial preferences',
    kind: 'MESSAGE',
    description: 'What you would like (and not like) at your funeral.',
    title: 'Funeral preferences',
    recommendedFor: 'Estate',
    body: `My preferences for what happens after I die.

These are preferences, not instructions. My family should decide what works for them.

Burial vs cremation:
  «BURIAL / CREMATION / EITHER / NO PREFERENCE»
  Reasoning: «OPTIONAL»

Funeral service:
  «RELIGIOUS / SECULAR / NONE»
  Where:    «LOCATION OR "wherever is easiest for the people there"»
  Music:    «SUGGESTIONS — or "no music" / "let people choose"»
  Readings: «SUGGESTIONS — or "none"»

Eulogist(s) I'd like to hear from, if they want to speak:
  1. «NAME»
  2. «NAME»

What to do with my ashes (if cremated):
  «SCATTER LOCATION / KEEP / FAMILY DECIDES»

In lieu of flowers:
  «DONATIONS TO X / FLOWERS ARE FINE / WHATEVER»

What I really don't want:
  «E.G. NO OPEN CASKET / NO HYMNS / NO LONG SPEECHES»

The most important thing: please don't argue about any of this. Whatever you choose is fine.`
  },

  {
    id: 'crypto_wallet_recovery',
    name: 'Crypto wallet recovery',
    kind: 'CREDENTIAL_BUNDLE',
    description: 'The seed phrases and recovery details for your crypto wallets.',
    title: 'Crypto wallet — recovery',
    recommendedFor: 'Estate+',
    body: `I hold cryptocurrency in the wallets below. The contents should pass under my will.

⚠ This Letter contains recovery secrets. The Recipient must treat it as critical
   information. Once they have these phrases, anyone with the phrase has the wallet.

Wallet 1 — «WALLET NAME, e.g. "Ledger Nano S — main holdings"»
  Type:           «HARDWARE / SOFTWARE / EXCHANGE»
  Network(s):     «BITCOIN / ETHEREUM / etc.»
  Approx holdings as of «DATE»: «AMOUNT»
  Recovery phrase (24 words):
    «WRITE THE PHRASE HERE — one word per line is fine»
  PIN (if hardware): «PIN»
  Notes: «ANYTHING THE RECIPIENT NEEDS TO KNOW»

Wallet 2 — «...»
  «(repeat for each wallet)»

Exchange accounts:
  Exchange: «KRAKEN / COINBASE / etc.»
  Login email: «EMAIL»
  2FA recovery: «SEE PASSWORD MANAGER BACKUP CODES»
  Approx holdings: «AMOUNT»

What I want done with these:
  «PASS TO X PER MY WILL / SELL AND CONVERT TO CASH / etc.»`
  },

  {
    id: 'subscriptions_to_cancel',
    name: 'Subscriptions to cancel',
    kind: 'ACTION',
    description: 'A list of recurring charges that will keep going unless someone stops them.',
    title: 'Subscriptions to cancel',
    recommendedFor: 'Draft',
    body: `Recurring charges on my accounts. Cancel these to stop the bills.

Streaming & media:
  - «NETFLIX — netflix.com/account, login: «EMAIL», ~$15/mo»
  - «SPOTIFY — ...»

Software & tools:
  - «...»

Cloud storage:
  - «ICLOUD — apple.com/account, ~$10/mo»
  - «GOOGLE ONE — ...»

Other:
  - «...»

For each one, sign in with the email/password in my password manager (recovery
details in the separate "Password manager — recovery" Letter), and cancel.

If you can't cancel, contact the card issuer to block the charges. The cards I
use for these are: «LAST 4 OF EACH CARD».`
  },

  {
    id: 'business_succession_brief',
    name: 'Business succession brief',
    kind: 'MESSAGE',
    description: 'For when you run a business and a successor needs to keep it running.',
    title: 'Business succession — «BUSINESS NAME»',
    recommendedFor: 'Estate+',
    body: `«BUSINESS NAME» — operating brief.

My role: «FOUNDER / DIRECTOR / etc.»

Immediate next steps (first 7 days):
  1. Notify the staff. They are not contractors, they are people who relied on this. The list is at «LINK». Pay this fortnight in full.
  2. Notify «BANK / ACCOUNTANT / etc.» of the change in control.
  3. The cash position as of «DATE» was «AMOUNT». «X MONTHS» of runway.

Key contacts:
  Accountant: «NAME» — «PHONE» — «EMAIL»
  Lawyer:     «NAME» — «PHONE» — «EMAIL»
  Bank:       «NAME» — «PHONE» — «ACCOUNT MANAGER»
  Landlord:   «NAME» — «PHONE» — «LEASE EXPIRES»
  Insurance:  «POLICY NUMBER, BROKER»

Outstanding obligations (as of «DATE»):
  - «...»

Key customers (~80% of revenue):
  - «...»

Recurring revenue:
  «MRR / ARR / RECURRING CONTRACTS»

What I'd like done:
  «SELL TO X / HAND TO Y / WIND DOWN / etc.»

My successor in this letter is: «NAME». They have agreed to this conversation
with me on «DATE». If they decline, the next-of-kin in my will makes the decision.`
  },

  {
    id: 'pet_care',
    name: 'Pet care',
    kind: 'ACTION',
    description: 'Who takes your pets, what they eat, who their vet is.',
    title: 'For my pets',
    recommendedFor: 'Draft',
    body: `My pets, in order of which needs the most attention.

«PET NAME» — «BREED / TYPE / AGE»
  Goes to:    «NEW HOME / NAME / PHONE»
  Backup:     «SECOND CHOICE»
  Vet:        «PRACTICE NAME / PHONE»
  Microchip:  «NUMBER»
  Insurance:  «POLICY / INSURER»
  Food:       «BRAND, AMOUNT, HOW OFTEN»
  Medication: «IF ANY»
  Important:  «KNOWN HEALTH ISSUES, BEHAVIOURS, FEARS»

«PET NAME 2» — «...»

A note: «PET NAME» is «X» years old and «PERSONALITY». They will be confused.
Please be patient with them.`
  },

  {
    id: 'final_social_posts',
    name: 'Final social posts',
    kind: 'MESSAGE',
    description: 'A goodbye post your executor can publish from your accounts.',
    title: 'Final post — please publish',
    recommendedFor: 'Estate',
    body: `If I'm gone, please publish the following from my accounts:

LinkedIn:
  «PROFESSIONAL FAREWELL — 3-4 sentences. Names of colleagues you want to thank,
   final sign-off.»

Other platforms (each):
  «PLATFORM — message»

Account access:
  Login details for each platform are in the "Password manager — recovery"
  Letter. To publish, log in normally; do not request memorialisation until
  after the posts are up.

After posting:
  - On Facebook, request memorialisation: facebook.com/help/contact/305593649477238
  - On Instagram, same flow.
  - On LinkedIn, "close account" via legal.linkedin.com/dpa.
  - On X, dormant accounts are auto-suspended in 30 days. No action needed.

Things I'd rather you NOT do:
  «E.G. DON'T POST TO TWITTER / DON'T MAKE A YOUTUBE TRIBUTE / etc.»`
  },

  {
    id: 'password_manager_recovery',
    name: 'Password manager — recovery',
    kind: 'CREDENTIAL_BUNDLE',
    description: 'How to get into your password manager. The skeleton key.',
    title: 'Password manager — recovery',
    recommendedFor: 'Estate',
    body: `My password manager is: «1PASSWORD / BITWARDEN / DASHLANE / etc.»

Master password: «MASTER PASSWORD»

Secret key / Account recovery key (1Password only): «KEY»

Two-factor:
  «AUTHENTICATOR APP on lost-phone is not recoverable. Backup codes are in the safe
   at «LOCATION» — pinned envelope marked "1Pass-Recovery".»

Login URL: «PASSWORD MANAGER URL»
Account email: «EMAIL USED FOR THE PASSWORD MANAGER»

Once inside, the items the family will most likely need are tagged "FAMILY"
in the manager. Everything else, my executor decides.

Note: please do NOT bulk-export all my passwords. Some of those accounts hold
material I'd rather stay sealed — work files, personal correspondence. Use the
tagged "FAMILY" items; ignore the rest.`
  },

  {
    id: 'organ_donation_preferences',
    name: 'Organ donation preferences',
    kind: 'MESSAGE',
    description: 'Your written wishes about organ and tissue donation.',
    title: 'Organ donation — my preferences',
    recommendedFor: 'Draft',
    body: `My wishes about organ and tissue donation.

I «AM / AM NOT» a registered organ donor on the «JURISDICTION» registry as of «DATE».

Specifically:
  Organs (heart, kidneys, liver, lungs, pancreas):  «YES / NO / FAMILY DECIDES»
  Tissue (corneas, skin, bone):                      «YES / NO / FAMILY DECIDES»
  Research donation:                                 «YES / NO / FAMILY DECIDES»

If my family is asked at the hospital, please confirm these wishes on my behalf.

My donor registration details are at: «WEBSITE / CARD LOCATION».`
  },

  {
    id: 'apology_or_unfinished',
    name: 'An apology, or an unfinished thing',
    kind: 'MESSAGE',
    description: 'For the things you didn\'t get to say while you were here.',
    title: 'For «PERSON\'S NAME»',
    recommendedFor: 'Estate',
    body: `«PERSON'S NAME»,

There are things I didn't say. Here they are.

«WHAT YOU NEED TO SAY. Whatever it is. The system holds it exactly. There is no
right length. There is no right tone. You don't have to be brave; you just have
to write it.»

I'm sorry it had to come this way.

— «YOUR NAME»`
  },

  {
    id: 'a_recipe',
    name: 'A recipe (or a list of them)',
    kind: 'MESSAGE',
    description: 'The food memory you want to pass on.',
    title: '«DISH NAME» — Mum/Dad/Nan\'s recipe',
    recommendedFor: 'Draft',
    body: `«DISH NAME»

This is how «I / WE» always made it.

Ingredients:
  «...»

Method:
  1. «...»
  2. «...»

Tips:
  - «THE THING ONLY YOU KNOW — e.g. "always cook the onions twice as long as the recipe says"»
  - «...»

I wanted you to have this. Make it for the people I would have made it for.

— «YOUR NAME»`
  },

  // ---------------------------------------------------------------------------
  // Time-capsule templates.
  //
  // These default to SCHEDULED_ONLY — they fire on the date you set, NOT on
  // a signal-triggered release. The point is that the recipient gets them
  // on the event date regardless of whether the Principal is still around.
  //
  // The compose UI surfaces this clearly: a time-capsule with a wedding
  // date in 2042 stays sealed until 2042 even if the Principal dies in
  // 2030, and is excluded from any signal-triggered Vault release.
  // ---------------------------------------------------------------------------

  {
    id: 'wedding_letter',
    name: 'For a wedding day',
    kind: 'MESSAGE',
    description:
      'A letter to read on a future wedding day — yours, your child\'s, anyone you might not be there for.',
    title: 'For «NAME»\'s wedding day',
    recommendedFor: 'Estate+',
    releaseMode: 'SCHEDULED_ONLY',
    body: `«NAME»,

If you're reading this on your wedding day, then today is a day I always hoped
I'd be in the room for. I'm not sure why I'm not — illness, distance, time —
but the fact that someone you love is handing you this means the people who
love you are still here, and that's the part that matters.

«WHAT YOU WANT THEM TO KNOW. A memory from when they were small. The thing
about marriage you wish someone had told you. A specific blessing for the
person they're choosing. Whatever you would have said in a toast.»

A small wish: be patient with each other in the years that look small from
inside the day. The big years take care of themselves.

I love you. I love you both.

— «YOUR NAME»`
  },

  {
    id: 'milestone_birthday',
    name: 'For a milestone birthday',
    kind: 'MESSAGE',
    description: 'A letter to open on an 18th, 21st, 30th, 50th, 80th — a birthday years away.',
    title: 'For your «NTH» birthday, «NAME»',
    recommendedFor: 'Estate',
    releaseMode: 'SCHEDULED_ONLY',
    body: `«NAME»,

Today you are «N». You probably already know what kind of person you've
become; this letter is from someone who knew the version of you that was
«AGE WHEN WRITTEN». Both are real.

«WHAT YOU WANT THEM TO KNOW. The thing you saw in them early. The advice
that's only useful at this age. The story you told them once when they
were little and want them to hear again.»

Happy birthday. I hope wherever you are today, the people who love you are
there with you.

— «YOUR NAME»`
  },

  {
    id: 'graduation',
    name: 'For a graduation',
    kind: 'MESSAGE',
    description: 'School, university, apprenticeship — a letter to be opened the day they finish.',
    title: 'On the day you graduate, «NAME»',
    recommendedFor: 'Estate',
    releaseMode: 'SCHEDULED_ONLY',
    body: `«NAME»,

Congratulations. I always thought you'd get here.

«WHAT YOU WANT THEM TO KNOW. The part you're proudest of. The part you
suspect was hardest for them. Whatever you would have told them at the
ceremony if you'd been the one giving the speech.»

A small piece of advice for the next part:
  «ONE LINE OF ADVICE. Not a paragraph. One line. The one that matters.»

— «YOUR NAME»`
  },

  {
    id: 'new_parent',
    name: 'For when they become a parent',
    kind: 'MESSAGE',
    description: 'For the day a child of yours becomes a parent themselves.',
    title: 'On becoming a parent, «NAME»',
    recommendedFor: 'Estate',
    releaseMode: 'SCHEDULED_ONLY',
    body: `«NAME»,

You have a child now. Welcome to the part of life where nothing prepares you
and somehow you do it anyway.

A few things I'd tell you, from someone who learned them about you:

  - Most days are ordinary. The ordinary is the point.
  - You will not be a perfect parent. That is fine. You will be the parent
    that this child needs, and "this child's parent" is the only job
    description that actually applies.
  - The cliché everyone says — that you'll love them more than you knew
    you could — turns out to be literally true. Let it.

«ANYTHING ELSE YOU WANT TO TELL THEM. The part of parenting you learned
late and wish you'd known. A specific blessing for the new arrival.»

Welcome to the chaos. I love you. I love them.

— «YOUR NAME»`
  },

  {
    id: 'anniversary',
    name: 'For a future anniversary',
    kind: 'MESSAGE',
    description: 'A letter to a partner, child, or friend on a date you want to mark from afar.',
    title: 'For «PERSON» on «DATE / OCCASION»',
    recommendedFor: 'Estate',
    releaseMode: 'SCHEDULED_ONLY',
    body: `«PERSON»,

I picked this date because «WHY THIS DATE — a wedding anniversary, the
anniversary of when we met, a birthday you want to mark from a future I
might not see».

«WHAT YOU WANT TO SAY. Specific. Particular. The thing only you know about
this date.»

I'm glad we had what we had.

— «YOUR NAME»`
  },

  {
    id: 'media_playlist',
    name: 'My music & playlists',
    kind: 'MEDIA_PLAYLIST',
    description: 'Share the Spotify playlists, YouTube channels, and songs that defined you.',
    title: 'My playlists — for you',
    releaseMode: 'SIGNAL_OR_SCHEDULED',
    body: `These are the songs and playlists I want you to have.

Spotify:
  «PLAYLIST OR ALBUM URL»
  «ARTIST NAME» — «WHAT THIS MEANS TO YOU»

YouTube:
  «PLAYLIST OR CHANNEL URL»
  «WHY I WANTED YOU TO HAVE THIS»

If any of these links no longer work, search for:
  «SONG / ARTIST / ALBUM NAME»
`,
  },

  {
    id: 'time_capsule_for_self',
    name: 'A time-capsule for yourself',
    kind: 'MESSAGE',
    description: 'A letter from you, now, to you, ten years from now. Recipient is your own email.',
    title: 'To future me — «N» years out',
    recommendedFor: 'Estate',
    releaseMode: 'SCHEDULED_ONLY',
    body: `Future me,

It's «TODAY'S DATE» as I write this. By the time you read it, you'll be
«AGE THEN». Some context from this end:

What's happening right now:
  «BRIEF SNAPSHOT — work, relationships, where I'm living, what's
   preoccupying me. Two paragraphs.»

What I expect to be true in «N» years:
  «WHAT I'M PREDICTING. Be specific. You can grade me later.»

What I want you to remember from this version of me:
  «THE PART YOU DON'T WANT TO LOSE TO TIME.»

Be kind to «N»-years-ago me. They were doing their best with what they
knew at the time.

— You (younger)`
  },
];

export function findTemplate(id: string): LetterTemplate | undefined {
  return LETTER_TEMPLATES.find((t) => t.id === id);
}

export function isTimeCapsule(t: LetterTemplate): boolean {
  return t.releaseMode === 'SCHEDULED_ONLY';
}
