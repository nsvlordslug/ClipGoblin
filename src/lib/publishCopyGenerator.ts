// ── Publish copy generator ──
// Generates platform-ready titles, captions, and hashtags from clip context.
// Deep differentiation by platform, tone, and content type.
// TODO(v2): Plug in Claude API for truly creative generation when API key is available.

export interface ClipContext {
  title: string
  eventTags: string[]
  emotionTags: string[]
  transcriptExcerpt?: string
  eventSummary?: string
  /** Full subtitle/transcript text — all dialogue from the clip */
  transcript?: string
  game?: string
  /** VOD title — often contains the game name */
  vodTitle?: string
  duration: number
  isMontage?: boolean
  clipCount?: number
  clipTitles?: string[]
}

// Frontend tones + backend mode names (backend modes are pass-through, not generated here)
export type CopyTone = 'punchy' | 'clean' | 'funny' | 'hype' | 'search' | 'minimal'
  | 'direct_quote' | 'blame' | 'internal_thought' | 'observation' | (string & {})

export interface GeneratedCopy {
  title: string
  description: string
  hashtags: string[]
  tone: CopyTone
}

// ── Content type classification ──

type ContentType = 'reaction' | 'gameplay' | 'fail' | 'clutch' | 'scare' | 'montage' | 'generic'

function classifyContent(ctx: ClipContext): ContentType {
  if (ctx.isMontage) return 'montage'
  const tags = [...ctx.eventTags, ...ctx.emotionTags].map(t => t.toLowerCase())
  if (tags.some(t => ['jumpscare', 'scare', 'shock', 'surprise'].includes(t))) return 'scare'
  if (tags.some(t => ['rage', 'frustration', 'fail', 'whiff', 'miss'].includes(t))) return 'fail'
  if (tags.some(t => ['clutch', 'escape', 'save', 'win'].includes(t))) return 'clutch'
  if (tags.some(t => ['scream', 'reaction', 'panic', 'hype'].includes(t))) return 'reaction'
  if (tags.some(t => ['chase', 'fight', 'kill', 'ambush', 'encounter'].includes(t))) return 'gameplay'
  return 'generic'
}

// ── Hook pools by content type ──

const HOOKS: Record<ContentType, string[]> = {
  reaction:  ['My reaction says it all', 'I LOST it at this part', 'Watch my face when this happens', 'The scream was involuntary', 'Pure unfiltered reaction', 'The way I screamed', 'My soul left my body', 'I was NOT ready', 'Caught my genuine reaction', 'The panic was real'],
  gameplay:  ['This play was TOO clean', 'Watch this play', 'Tell me this isn\'t peak gaming', 'The most intense moment', 'Nobody expected this', 'Calculated', 'They didn\'t see it coming', 'This is what cracked looks like', 'The play of my life', 'Absolutely surgical'],
  fail:      ['Pain. Just pain', 'How did I mess this up', 'The universe said NO', 'I really thought I had it', 'Worst timing in history', 'Certified bot moment', 'I peaked but in the wrong direction', 'Uninstalling after this', 'This is my villain origin story', 'Down astronomically'],
  clutch:    ['CLUTCH of the century', 'Down bad to comeback king', 'Against all odds somehow', 'They thought it was over', 'The clutch factor was UNREAL', 'Heart was pounding for this one', 'Snatching victory from the jaws of defeat', '1HP and a dream', 'Refuse to lose', 'When the plot armor kicks in'],
  scare:     ['I literally jumped out of my chair', 'DO NOT watch this alone', 'The jumpscare to end all jumpscares', 'My heart actually stopped', 'I\'m never playing this again', 'I need new pants after this', 'Pure terror caught on cam', 'My fight or flight chose neither', 'This game hates me', 'Almost threw my mouse'],
  montage:   ['Best moments from the stream', 'Every clip hit different', 'Highlight reel goes crazy', 'The stream was WILD today', 'These moments were TOO good not to compile', 'Nothing but chaos this session', 'Stream moments that live rent-free', 'When the content writes itself'],
  generic:   ['You need to see this', 'This moment hits different', 'Stream moment of the day', 'Caught on stream', 'Wait for it...', 'Didn\'t expect this one', 'This clip is unreal', 'How did this happen', 'The things that happen on stream', 'Tell me this isn\'t content'],
}

// ── Title template pools for standalone generation (no pipe characters) ──

/** Title templates organized by content type. {game} and {action} are placeholders.
 *  Each pool has 15+ entries so repeated rerolls feel fresh. */
const TITLE_TEMPLATES: Record<ContentType, { withGame: string[]; noGame: string[] }> = {
  clutch: {
    withGame: [
      'Insane Clutch in {game}', 'When the Clutch Gene Activates in {game}',
      '{game} Clutch That Shouldn\'t Have Worked', 'Against All Odds in {game}',
      'The {game} Clutch of a Lifetime', '1HP Clutch in {game}',
      'Never Count Me Out in {game}', 'They Thought I Was Done in {game}',
      '{game} Comeback for the Ages', 'How I Clutched This in {game}',
      'Refusing to Lose in {game}', 'Pulled Off the Impossible in {game}',
      'Down Bad to Winning in {game}', 'The Clutch That Had Chat Screaming in {game}',
      'My Best Clutch Ever in {game}', 'Plot Armor Activated in {game}',
    ],
    noGame: [
      'The Clutch of a Lifetime', 'Against All Odds', '1HP and a Dream',
      'When the Clutch Gene Activates', 'They Thought I Was Done',
      'Never Count Me Out', 'Comeback for the Ages', 'How Did I Pull This Off',
      'Down Bad to Winning', 'The Clutch That Had Chat Screaming',
      'My Best Clutch Ever', 'Refused to Lose', 'Plot Armor Moment',
      'Snatching Victory', 'Heart Was POUNDING for This One',
    ],
  },
  fail: {
    withGame: [
      'I Swear {game} Hates Me', '{game} Said Absolutely Not',
      'My Worst {game} Moment', 'POV You\'re the Potato in {game}',
      '{game} Humbled Me', 'Why Do I Still Play {game}',
      'The Most Embarrassing {game} Clip', 'I Need a Break From {game}',
      '{game} Really Did Me Like That', 'This {game} Fail Hurts to Watch',
      'Down Bad in {game}', 'Certified Bot Moment in {game}',
      'I Peaked in the Wrong Direction in {game}', '{game} Gave Me Trust Issues',
      'Uninstalling {game} After This', 'My Villain Origin Story in {game}',
    ],
    noGame: [
      'This Fail Hurts to Watch', 'How Did I Mess This Up',
      'Certified Bot Moment', 'Pain and Suffering on Stream',
      'I Peaked in the Wrong Direction', 'Embarrassing Clip of the Day',
      'The Universe Said No', 'Down Astronomically',
      'My Villain Origin Story', 'Worst Timing in History',
      'Why Does This Keep Happening', 'I Should Have Gone Outside',
      'I Really Thought I Had It', 'The Game Humbled Me',
      'Uninstalling After This', 'Trust Issues After This Play',
    ],
  },
  scare: {
    withGame: [
      '{game} Almost Gave Me a Heart Attack', 'Jump Scare in {game} Got Me',
      'I\'m Never Playing {game} Again', '{game} at 3am Was a Mistake',
      'This {game} Moment Made Me Scream', 'DO NOT Play {game} Alone',
      '{game} Developers Are Evil', 'The Scariest Moment in {game}',
      'My Heart Stopped Playing {game}', '{game} Horror Hit Different',
      'Almost Threw My Mouse Playing {game}', 'Pure Terror in {game}',
      '{game} Is NOT for the Faint of Heart', 'Fight or Flight in {game}',
      'The Jumpscare That Ended Me in {game}', '{game} Got Me Screaming',
    ],
    noGame: [
      'Almost Had a Heart Attack', 'This Jump Scare Got Me Good',
      'I\'m Never Playing This Again', 'DO NOT Watch This Alone',
      'Horror Gaming at 3am Was a Mistake', 'My Heart Actually Stopped',
      'The Developers Are Evil', 'The Scariest Moment',
      'Almost Threw My Mouse', 'Pure Terror on Stream',
      'I Need New Pants After This', 'Fight or Flight Chose Neither',
      'The Jumpscare That Ended Me', 'Got Me Screaming on Stream',
      'This Game Is NOT for the Faint of Heart', 'My Soul Left My Body',
    ],
  },
  reaction: {
    withGame: [
      'My Honest Reaction to {game}', 'This {game} Moment Broke Me',
      'Watch My Face When This Happens in {game}', '{game} Had Me Screaming',
      'The {game} Moment That Got Me', 'I Was NOT Ready for This in {game}',
      'Genuine Reaction to {game}', 'You Can See the Exact Moment in {game}',
      '{game} Hits Different When This Happens', 'Stream Reaction to {game}',
      'The Way I Reacted to This in {game}', '{game} Gave Me Whiplash',
      'Caught My Genuine Reaction Playing {game}', 'The Panic When {game} Does This',
      'Pure Emotion Playing {game}', 'Live Reaction to {game} Chaos',
    ],
    noGame: [
      'My Honest Reaction', 'This Moment Broke Me',
      'Watch My Face When This Happens', 'I Was NOT Ready',
      'Genuine Reaction Caught on Stream', 'You Can See the Exact Moment',
      'Stream Reaction of the Day', 'The Way I Reacted',
      'Pure Emotion on Camera', 'The Panic Was Real',
      'Caught My Genuine Reaction', 'I Lost It at This Part',
      'The Scream Was Involuntary', 'My Soul Left My Body',
      'The Things That Happen on Stream', 'Whiplash Moment on Stream',
    ],
  },
  gameplay: {
    withGame: [
      'Clean Play in {game}', 'They Weren\'t Ready for This in {game}',
      'This {game} Play Was Surgical', 'Peak {game} Right Here',
      'The Most Intense {game} Moment', 'Nobody Expected This in {game}',
      'Calculated Everything in {game}', 'This Is Why I Play {game}',
      '{game} Gameplay at Its Peak', 'The Play That Had Chat Going Crazy in {game}',
      'Cracked at {game}', 'Built Different in {game}',
      'When Everything Lines Up in {game}', 'My Cleanest {game} Play',
      'The {game} Highlight of the Day', 'Top Tier {game} Gameplay',
    ],
    noGame: [
      'This Play Was Too Clean', 'They Weren\'t Ready',
      'Surgical Precision', 'Peak Gaming Right Here',
      'The Most Intense Moment', 'Nobody Expected This',
      'Calculated Everything', 'This Is Why I Game',
      'Gameplay at Its Peak', 'The Play That Had Chat Going Crazy',
      'Absolutely Cracked', 'Built Different', 'When Everything Lines Up',
      'My Cleanest Play', 'Highlight of the Day', 'Top Tier Gameplay',
    ],
  },
  montage: {
    withGame: [
      'Best {game} Moments This Stream', '{game} Stream Highlights',
      'Nothing but {game} Chaos', '{game} Moments That Live Rent-Free',
      'When {game} Content Writes Itself', 'The Best of {game} Today',
      '{game} Highlight Reel', 'Peak {game} in One Video',
    ],
    noGame: [
      'Best Moments This Stream', 'Stream Highlights',
      'Nothing but Chaos This Session', 'Moments That Live Rent-Free',
      'When the Content Writes Itself', 'The Best of Today\'s Stream',
      'Highlight Reel Goes Crazy', 'Peak Gaming in One Video',
    ],
  },
  generic: {
    withGame: [
      'This Happened in {game}', '{game} Moment That Hits Different',
      'Didn\'t Expect This in {game}', 'Stream Moment Playing {game}',
      'When {game} Gets Wild', 'Only in {game}',
      '{game} Never Gets Old', 'The Things That Happen in {game}',
      'Tell Me This Isn\'t Content from {game}', 'Classic {game} Moment',
      'Wait For It in {game}', 'You Need to See This {game} Clip',
      '{game} Hits Different on Stream', '{game} Doing {game} Things',
      'Just Another Day in {game}', 'How Did This Happen in {game}',
    ],
    noGame: [
      'This Moment Hits Different', 'Didn\'t Expect This One',
      'Stream Moment of the Day', 'When Things Get Wild',
      'Only on Stream', 'Gaming Never Gets Old',
      'The Things That Happen on Stream', 'Tell Me This Isn\'t Content',
      'Wait For It', 'You Need to See This Clip',
      'Stream Hits Different', 'Just Another Day Gaming',
      'How Did This Happen', 'Caught on Stream',
      'This Clip Is Unreal', 'Gaming Doing Gaming Things',
    ],
  },
}

// ── Caption template pools (15-20+ per tone, factoring in game/transcript/content type) ──

type CaptionTemplate = (ctx: { game?: string; quote?: string; action: string; ctype: ContentType; title?: string }) => string

const CAPTION_POOLS: Record<string, CaptionTemplate[]> = {
  funny: [
    ({ game }) => game ? `I swear ${game} hates me` : 'I swear this game hates me',
    ({ game }) => game ? `POV: you're the potato in ${game}` : 'POV: you\'re the potato',
    ({ game }) => game ? `${game} really said "absolutely not" today` : 'The game really said "absolutely not" today',
    ({ game }) => game ? `I need a therapist and it's ${game}'s fault` : 'I need a therapist and gaming is the reason',
    ({ quote }) => quote ? `"${quote}" — famous last words` : 'Famous last words on stream',
    ({ quote }) => quote ? `"${quote}" — narrator: it did not, in fact, work out` : 'Narrator: it did not, in fact, work out',
    ({ game }) => game ? `Why do I keep coming back to ${game}` : 'Why do I keep doing this to myself',
    ({ game }) => game ? `${game} owes me an apology and a refund` : 'This game owes me an apology',
    ({ game }) => game ? `Streaming ${game} is a cry for help at this point` : 'Streaming is a cry for help at this point',
    ({ ctype }) => ctype === 'fail' ? 'Chat was right about me the whole time' : 'Sometimes the content makes itself',
    ({ game }) => game ? `Every day I choose ${game} and every day I suffer` : 'Every day I choose violence and every day I suffer',
    ({ ctype }) => ctype === 'scare' ? 'Might need new pants after that one' : 'I should have gone outside today',
    ({ game }) => game ? `${game} matchmaking doing me dirty again` : 'Matchmaking doing me dirty again',
    ({ quote }) => quote ? `"${quote}" — and then it got worse` : 'And then it got worse',
    ({ game }) => game ? `I peaked in ${game} but in the wrong direction` : 'I peaked but in the wrong direction',
    ({ ctype, game }) => ctype === 'fail' ? (game ? `Certified bot moment in ${game}` : 'Certified bot moment') : 'Chat is going to roast me for this',
    ({ game }) => game ? `${game} just gave me trust issues` : 'This game just gave me trust issues',
    ({ game }) => game ? `My ${game} highlight reel but it's all pain` : 'My highlight reel but it\'s all pain',
    ({ ctype }) => ctype === 'scare' ? 'My neighbors definitely heard that scream' : 'I need to take a break after this one',
    ({ game }) => game ? `Uninstalling ${game} (again)` : 'Uninstalling (again)',
  ],
  hype: [
    ({ game }) => game ? `THIS is why I play ${game}` : 'THIS is why I game',
    ({ game }) => game ? `They weren't ready for this one in ${game}` : 'They weren\'t ready for this one',
    ({ quote }) => quote ? `"${quote}" — DIFFERENT BREED` : 'DIFFERENT BREED',
    ({ game }) => game ? `${game} but I'm actually cracked` : 'Actually cracked at this',
    ({ ctype }) => ctype === 'clutch' ? 'DOWN BAD TO COMEBACK KING' : 'BUILT DIFFERENT',
    ({ game }) => game ? `Peak ${game} performance right here` : 'Peak performance right here',
    ({ game }) => game ? `Nobody does it like this in ${game}` : 'Nobody does it like this',
    ({ ctype }) => ctype === 'clutch' ? 'Heart was POUNDING for this one' : 'WE DON\'T MISS',
    ({ game }) => game ? `${game} clip of the year right here` : 'Clip of the year right here',
    ({ quote }) => quote ? `"${quote}" — UNSTOPPABLE` : 'UNSTOPPABLE',
    ({ game }) => game ? `This ${game} play goes CRAZY` : 'This play goes CRAZY',
    ({ game }) => game ? `${game} players wish they could do this` : 'They wish they could do this',
    ({ ctype }) => ctype === 'gameplay' ? 'That was absolutely SURGICAL' : 'THEY CAN\'T STOP US',
    ({ game }) => game ? `Woke up and chose violence in ${game}` : 'Woke up and chose violence',
    ({ game }) => game ? `${game} moment that goes down in history` : 'Moment that goes down in history',
    ({ ctype, game }) => ctype === 'clutch' ? (game ? `Refuse to lose in ${game}` : 'Refuse to lose') : 'NO ONE DOES IT BETTER',
    ({ game }) => game ? `The ${game} gods were on my side` : 'The gaming gods were on my side',
    ({ quote }) => quote ? `"${quote}" — GOATED` : 'GOATED',
    ({ game }) => game ? `${game} but I have plot armor` : 'Plot armor activated',
    ({ ctype }) => ctype === 'gameplay' ? 'Calculated. Everything was calculated.' : 'WE GO CRAZY ON STREAM',
  ],
  // ────────────────────────────────────────────────────────────────
  // QUOTE: Always centers on a short quoted phrase. Structure:
  //   "[short quote]" + reaction/commentary
  // About highlighting something that was SAID.
  // ────────────────────────────────────────────────────────────────
  direct_quote: [
    ({ quote, game }) => quote ? `"${quote}" — ${game || 'gaming'} brings out the truth` : '',
    ({ quote, game }) => quote && game ? `said "${quote}" and the whole ${game} lobby felt it` : '',
    ({ quote, game }) => quote && game ? `"${quote}" hits different at 2am playing ${game}` : '',
    ({ quote, game }) => quote ? `"${quote}" — narrator: the ${game || 'lobby'} was not ready` : '',
    ({ quote }) => quote ? `"${quote}" — famous last words` : '',
    ({ quote }) => quote ? `"${quote}" and then everything went sideways` : '',
    ({ quote }) => quote ? `said "${quote}" with full confidence. it did not end well.` : '',
    ({ quote }) => quote ? `"${quote}" — the exact moment of realization` : '',
    ({ quote, game }) => quote ? `"${quote}" — just ${game || 'streamer'} things` : '',
    ({ quote }) => quote ? `"${quote}" — and I will NOT be taking that back` : '',
    ({ quote, game }) => quote ? `"${quote}" — ${game || 'this game'} made me say that out loud` : '',
    ({ quote }) => quote ? `"${quote}" — the voice crack says it all` : '',
    ({ quote }) => quote ? `"${quote}" — audio ON for this one` : '',
    ({ quote }) => quote ? `"${quote}" — chat went absolutely feral` : '',
    ({ quote, game }) => quote && game ? `"${quote}" — a normal day in ${game}` : '',
    ({ quote }) => quote ? `"${quote}" and I meant every single word` : '',
    ({ quote }) => quote ? `"${quote}" — you can hear the soul leaving the body` : '',
    // Fallbacks when no transcript — use title as the "quote"
    ({ title, game }) => title ? `"${title}" — the clip speaks for itself` : game ? `${game} left me speechless` : 'The clip speaks for itself',
    ({ game }) => game ? `No words can describe this ${game} moment. Just watch.` : 'No words. Just watch.',
    ({ title }) => title ? `Turn the audio on for "${title}". Trust.` : 'Turn the audio on. Trust.',
  ],
  // ────────────────────────────────────────────────────────────────
  // BLAME: Never uses quotes. Structure:
  //   Blame-focused commentary (teammates, game, matchmaking, lag, self)
  // About WHO or WHAT is at fault.
  // ────────────────────────────────────────────────────────────────
  blame: [
    ({ game }) => game ? `${game} matchmaking looked at my win rate and chose violence` : 'Matchmaking looked at my win rate and chose violence',
    ({ game }) => game ? `my teammates spectating like it's a ${game} movie theater` : 'My teammates spectating like it\'s a movie theater',
    ({ game }) => game ? `I blame ${game} for this one. Not me. Never me.` : 'I blame the game for this one. Not me. Never me.',
    ({ game }) => game ? `${game} said it's MY turn to be the bot` : 'The game said it\'s MY turn to be the bot',
    ({ game }) => game ? `dying like this in ${game} should be a criminal offense` : 'Dying like this should be a criminal offense',
    ({ game }) => game ? `solo queue ${game} is a federal crime` : 'Solo queue is a federal crime',
    ({ game }) => game ? `${game} hit reg saw what I did and said nah` : 'Hit reg saw what I did and said nah',
    ({ game }) => game ? `filing an emotional damage report against ${game}` : 'Filing an emotional damage report against this game',
    ({ game }) => game ? `my ${game} teammates are running a spectating-only build` : 'My teammates are running a spectating-only build',
    ({ game }) => game ? `${game} devs owe me a formal apology and a therapy session` : 'The devs owe me a formal apology and a therapy session',
    ({ game }) => game ? `how am I the problem when ${game} matchmaking did this to me` : 'How am I the problem when matchmaking did this to me',
    ({ game }) => game ? `${game} decided I was the content today. I did not consent.` : 'The game decided I was the content today. I did not consent.',
    ({ game }) => game ? `I deserve a higher rank in ${game} and this clip is exhibit A` : 'I deserve a higher rank and this clip is exhibit A',
    ({ game }) => game ? `${game} put me in the blender and hit puree` : 'The game put me in the blender and hit puree',
    ({ game }) => game ? `my entire ${game} team chose violence against ME specifically` : 'My entire team chose violence against ME specifically',
    ({ game }) => game ? `${game} RNG hates me and at this point it's personal` : 'RNG hates me and at this point it\'s personal',
    ({ game }) => game ? `whoever balanced ${game} owes me money` : 'Whoever balanced this game owes me money',
    ({ game }) => game ? `it was the ${game} servers. it's always the servers.` : 'It was the servers. It\'s always the servers.',
    ({ game }) => game ? `my ${game} team playing like they're speedrunning a loss` : 'My team playing like they\'re speedrunning a loss',
    ({ game }) => game ? `${game} netcode is my villain origin story` : 'The netcode is my villain origin story',
  ],
  // ────────────────────────────────────────────────────────────────
  // THOUGHT: Never uses quotes. Structure:
  //   Reflective, philosophical, or dramatic internal monologue
  // About what you're THINKING or FEELING.
  // ────────────────────────────────────────────────────────────────
  internal_thought: [
    ({ game }) => game ? `sometimes you queue up for ${game} and ${game} queues up for you` : 'Sometimes you queue up and the game queues up for you',
    ({ game }) => game ? `there's a fine line between confidence and foolishness. ${game} found it.` : 'There\'s a fine line between confidence and foolishness and I live on it',
    ({ game }) => game ? `${game} really said "character development"` : 'The game really said character development',
    ({ game }) => game ? `sometimes you're the highlight reel, sometimes you're the potato. ${game} decides.` : 'Sometimes you\'re the highlight. Sometimes you\'re the potato.',
    ({ game }) => game ? `my brain during ${game}: stop. me: one more game.` : 'My brain: stop. Me: one more game.',
    ({ game }) => game ? `every ${game} session starts with hope and ends exactly like this` : 'Every session starts with hope and ends exactly like this',
    ({ game }) => game ? `${game} is just therapy but the copay is your dignity` : 'Gaming is just therapy but the copay is your dignity',
    ({ game }) => game ? `me pretending I'm fine after that ${game} round` : 'Me pretending I\'m fine after that round',
    ({ game }) => game ? `my ${game} arc is just suffering with better graphics` : 'My gaming arc is just suffering with better graphics',
    ({ game }) => game ? `the inner dialogue of a ${game} player would get you banned` : 'My inner dialogue would get me banned from twitch',
    ({}) => 'I think about this play when I can\'t sleep',
    ({}) => 'quiet on the outside. absolute chaos on the inside.',
    ({}) => 'externally calm. internally: AAAAAAA.',
    ({}) => 'some moments teach you something. this was not one of them.',
    ({}) => 'I saw it coming. I watched it happen. I did nothing.',
    ({}) => 'this clip lives rent free in my head',
    ({ ctype }) => ctype === 'scare' ? 'my brain screamed run. my hands disagreed.' : 'the thoughts during this clip were not stream-appropriate',
    ({ ctype }) => ctype === 'clutch' ? 'acting like I planned that. I did not plan that.' : 'still processing what just happened honestly',
    ({ game }) => game ? `I didn't choose the ${game} life. the ${game} life chose poorly.` : 'I didn\'t choose the gaming life. The gaming life chose poorly.',
    ({ game }) => game ? `${game} gave me an existential crisis and called it gameplay` : 'This game gave me an existential crisis and called it gameplay',
  ],
  observation: [
    ({ game, action }) => game ? `Watch the exact moment it all goes wrong in ${game}` : `Watch the exact moment it all goes wrong`,
    ({ game }) => game ? `This is what peak ${game} content looks like` : 'This is what peak content looks like',
    ({ quote }) => quote ? `The way they said "${quote}" — you can feel it` : 'You can feel the emotion in this one',
    ({ game }) => game ? `A perfectly normal ${game} stream` : 'A perfectly normal stream',
    ({ ctype }) => ctype === 'scare' ? 'Jump scare at its finest' : 'You have to watch this twice to catch everything',
    ({ game }) => game ? `${game} creating content by itself` : 'The game creates the content',
    ({ ctype, game }) => ctype === 'clutch' ? (game ? `Watch the clutch factor kick in during ${game}` : 'Watch the clutch factor kick in') : 'Sometimes the stars align',
    ({ game }) => game ? `Notice how quickly things went sideways in ${game}` : 'Notice how quickly things went sideways',
    ({ quote }) => quote ? `"${quote}" — the tone shift is everything` : 'The tone shift is everything',
    ({ game }) => game ? `This ${game} clip is a whole movie` : 'This clip is a whole movie',
    ({ ctype }) => ctype === 'fail' ? 'Study this clip. Learn from it. Don\'t repeat it.' : 'The content made itself today',
    ({ game }) => game ? `${game} gameplay but it's a Netflix series` : 'Gameplay but it\'s a Netflix series',
    ({ ctype }) => ctype === 'reaction' ? 'That reaction was 100% genuine' : 'Real and unscripted, as always',
    ({ game }) => game ? `A masterclass in ${game}... or the opposite` : 'A masterclass... or the opposite',
    ({ game }) => game ? `${game} never fails to deliver moments like this` : 'Never a dull moment on stream',
  ],
  punchy: [
    ({ game, action }) => game ? `${action} in ${game}. No further context needed.` : `${action}. No further context needed.`,
    ({ game }) => game ? `${game} hits different on stream` : 'Hits different on stream',
    ({ quote }) => quote ? `"${quote}" — nuff said` : 'Clip speaks for itself',
    ({ game }) => game ? `Only in ${game}` : 'Only on stream',
    ({ ctype }) => ctype === 'clutch' ? 'Ice in the veins for this one' : 'The timing on this was insane',
    ({ game }) => game ? `${game}. That's it. That's the caption.` : 'That\'s it. That\'s the caption.',
    ({ game }) => game ? `Live ${game} moment. No script needed.` : 'No script needed',
    ({ quote }) => quote ? `"${quote}"` : 'Sometimes the clip is enough',
    ({ game }) => game ? `Peak ${game} content right here` : 'Peak content right here',
    ({ ctype }) => ctype === 'fail' ? 'It was over before it started' : 'This clip is going places',
    ({ game }) => game ? `${game} gave us this one for free` : 'Free content',
    ({ game }) => game ? `Just a normal ${game} moment` : 'Just a normal gaming moment',
    ({ ctype }) => ctype === 'scare' ? 'Volume warning.' : 'Watch this. Trust me.',
    ({ game }) => game ? `When ${game} writes the script` : 'When the game writes the script',
    ({ ctype }) => ctype === 'clutch' ? 'Main character energy' : 'Couldn\'t make this up',
  ],
  clean: [
    ({ game }) => game ? `Playing ${game} live on Twitch` : 'Live on Twitch',
    ({ game, action }) => game ? `${action} in ${game}. Caught live on stream.` : `${action}. Caught live on stream.`,
    ({ game }) => game ? `${game} stream moment` : 'Stream moment',
    ({ game }) => game ? `Another day, another ${game} clip` : 'Another day, another stream clip',
    ({ game }) => game ? `From today's ${game} stream` : 'From today\'s stream',
    ({ game }) => game ? `${game} gameplay live on Twitch` : 'Gameplay live on Twitch',
    ({ quote }) => quote ? `"${quote}" — live on stream` : 'Caught live on stream',
    ({ game }) => game ? `Streaming ${game}. Follow for more.` : 'Follow for more stream clips',
    ({ game }) => game ? `${game} clip from today's session` : 'Clip from today\'s session',
    ({ game }) => game ? `Live ${game} moment. Link in bio.` : 'Live stream moment. Link in bio.',
    ({ game }) => game ? `${game} highlights from the stream` : 'Highlights from the stream',
    ({ game }) => game ? `Caught this one live playing ${game}` : 'Caught this one live',
    ({ game }) => game ? `Today on ${game}` : 'Today on stream',
    ({ game }) => game ? `${game} on Twitch. Clips like this every stream.` : 'Clips like this every stream on Twitch',
    ({ game }) => game ? `${game} content from the archives` : 'Content from the archives',
  ],
  search: [
    ({ game, action }) => game ? `${game} ${action.toLowerCase()} stream highlights` : `${action.toLowerCase()} stream highlights`,
    ({ game }) => game ? `${game} best moments stream clips Twitch` : 'Best moments stream clips Twitch',
    ({ game }) => game ? `${game} gameplay highlights live stream` : 'Gameplay highlights live stream',
    ({ game }) => game ? `${game} funny moments and best plays` : 'Funny moments and best plays on stream',
    ({ game }) => game ? `${game} stream clips that go hard` : 'Stream clips that go hard',
  ],
  minimal: [
    ({ quote }) => quote ? `"${quote}"` : '',
    ({ game }) => game ? `${game}` : '',
    ({ game }) => game ? `${game} moment` : 'Stream moment',
    ({ game, action }) => game ? `${action}. ${game}.` : action,
    ({}) => 'Live on Twitch',
  ],
}

// ── Platform-specific caption builders ──

const FUNNY_TAILS = [
  'streaming is a lifestyle', 'streaming is pain', 'i need therapy after this',
  'why does this keep happening', 'this game owes me an apology',
  'i should have gone outside today', 'chat was right about me',
  'i make these mistakes so you don\'t have to', 'at least chat was entertained',
]
const HYPE_TAILS = [
  'DIFFERENT BREED', 'WE DON\'T MISS', 'BUILT DIFFERENT',
  'THEY CAN\'T STOP US', 'UNSTOPPABLE', 'NO ONE DOES IT BETTER',
  'GOATED', 'CRACKED OUT OF MY MIND', 'WOKE UP DANGEROUS',
]

function buildTikTokCaption(hook: string, ctx: ClipContext, tone: CopyTone, ctype: ContentType, variation = 0): string {
  const game = ctx.game
  const hasGame = !!game
  const transcriptBit = extractTranscriptHook(ctx.transcript || ctx.transcriptExcerpt, variation)
  const hasTranscript = !!transcriptBit
  const pick = <T,>(arr: T[]) => arr[variation % arr.length]
  const action = describeAction(ctype, ctx).toLowerCase()
  const gameTag = hasGame ? ` #${game!.replace(/[^a-zA-Z0-9]/g, '')}` : ''

  switch (tone) {
    case 'punchy': {
      if (hasTranscript && hasGame) return `"${transcriptBit}" — ${action} in ${game}`
      if (hasTranscript) return `"${transcriptBit}" — ${hook}`
      if (hasGame) return `${hook} — ${game} hits different`
      return hook
    }
    case 'clean': {
      const base = hasGame ? `${hook}. Playing ${game} live on Twitch.` : `${hook}. Live on Twitch.`
      return hasTranscript ? `"${transcriptBit}"\n\n${base}` : base
    }
    case 'funny': {
      const tail = pick(FUNNY_TAILS)
      if (hasTranscript && hasGame) return `"${transcriptBit}" — ${game} really said no. ${tail}`
      if (hasTranscript) return `"${transcriptBit}" — ${tail}`
      if (hasGame && ctype === 'fail') return `${hook} — ${game} owes me an apology`
      if (hasGame && ctype === 'scare') return `${hook} — never playing ${game} again`
      if (hasGame) return `${hook} — ${tail}`
      return `${hook} — ${tail}`
    }
    case 'hype': {
      const tail = pick(HYPE_TAILS)
      if (hasTranscript && hasGame) return `"${transcriptBit}" — ${game} ${tail}`
      if (hasTranscript) return `"${transcriptBit}" — ${tail}`
      if (hasGame) return `${hook} in ${game} — ${tail}`
      return `${hook} — ${tail}`
    }
    case 'search': {
      return `${hook}${gameTag} #gaming #fyp`
    }
    case 'minimal': {
      if (hasTranscript) return `"${transcriptBit}"`
      if (hasGame) return `${hook} — ${game}`
      return hook
    }
    default: return hook
  }
}

function buildMontageContentSummary(ctx: ClipContext): string {
  const parts: string[] = []
  const events = ctx.eventTags.map(t => t.replace(/-/g, ' '))
  const emotions = ctx.emotionTags.map(t => t.replace(/-/g, ' '))

  if (events.some(t => ['chase', 'escape', 'fight'].some(e => t.includes(e)))) parts.push('intense chases')
  if (events.some(t => ['kill', 'ambush'].some(e => t.includes(e)))) parts.push('clutch kills')
  if (events.some(t => t.includes('jumpscare'))) parts.push('jumpscares')
  if (emotions.some(t => ['rage', 'frustration'].some(e => t.includes(e)))) parts.push('rage moments')
  if (emotions.some(t => ['panic', 'shock'].some(e => t.includes(e)))) parts.push('reactions')
  if (events.some(t => t.includes('scream'))) parts.push('screaming')
  if (events.some(t => t.includes('encounter'))) parts.push('wild encounters')

  if (parts.length === 0) parts.push('highlights', 'best moments')
  return parts.slice(0, 4).join(', ')
}

function buildYouTubeCaption(hook: string, ctx: ClipContext, tone: CopyTone, ctype: ContentType): string {
  const game = ctx.game
  const hasGame = !!game
  const year = new Date().getFullYear()
  const action = describeAction(ctype, ctx)
  const gameLine = hasGame ? `\nGame: ${game}` : ''

  if (ctype === 'montage') {
    const summary = buildMontageContentSummary(ctx)
    const clipCount = ctx.clipCount || 0
    const durMin = Math.round(ctx.duration / 60)

    switch (tone) {
      case 'punchy':
        return hasGame
          ? `${hook}\n\n${clipCount} clips of pure ${game} chaos — ${summary}.\n\nCaught live on Twitch. Subscribe for more!`
          : `${hook}\n\n${clipCount} clips of pure chaos — ${summary}.\n\nCaught live on Twitch. Subscribe for more!`
      case 'clean':
        return `Stream highlights compilation featuring ${summary}.\n\n${clipCount} clips • ${durMin} minutes${gameLine}\nStream: twitch.tv/[channel]`
      case 'funny':
        return hasGame
          ? `${hook}\n\nThis ${game} stream had everything: ${summary}... and suffering.\nI need a break.`
          : `${hook}\n\nThis stream had everything: ${summary}... and suffering.\nI need a break.`
      case 'hype':
        return hasGame
          ? `${hook}\n\nTHIS ${game!.toUpperCase()} SESSION WAS UNREAL.\n${clipCount} moments of absolute madness — ${summary}.\n\nLike + Sub for more highlights`
          : `${hook}\n\nTHIS SESSION WAS UNREAL.\n${clipCount} moments of absolute madness — ${summary}.\n\nLike + Sub for more highlights`
      case 'search':
        return hasGame
          ? `${ctx.title}\n\n${game} stream highlights and best moments ${year}. Featuring ${summary}.\n\n${clipCount} clips • ${durMin}min compilation${gameLine}\nRecorded live on Twitch`
          : `${ctx.title}\n\nStream highlights and best moments ${year}. Featuring ${summary}.\n\n${clipCount} clips • ${durMin}min compilation\nRecorded live on Twitch`
      case 'minimal':
        return hasGame ? `Best moments from today's ${game} stream. ${summary}.` : `Best moments from today's stream. ${summary}.`
      default: return hook
    }
  }

  // Single clip
  switch (tone) {
    case 'punchy':
      return hasGame
        ? `${hook}\n\n${action} in ${game} — caught live on Twitch.\nFollow for more highlights!`
        : `${hook}\n\nCaught live on stream. Follow for more highlights!`
    case 'clean':
      return `${hook} — caught live on stream.${gameLine}\nStream: twitch.tv/[channel]`
    case 'funny':
      return hasGame
        ? `${hook}\n\n${game} really said "not today." I still can't believe this happened.\n\nStreaming live on Twitch!`
        : `${hook}\n\nI still can't believe this happened. Streaming live on Twitch!`
    case 'hype':
      return hasGame
        ? `${hook}\n\n${action.toUpperCase()} IN ${game!.toUpperCase()}. UNREAL.\n\nLike + Sub for more highlights`
        : `${hook}\n\n${action.toUpperCase()}. UNREAL.\n\nLike + Sub for more highlights`
    case 'search':
      return hasGame
        ? `${ctx.title}\n\n${game} gameplay — ${action.toLowerCase()}.\n${gameLine}\nRecorded live on Twitch`
        : `${ctx.title}\n\nGameplay highlights.\nRecorded live on Twitch`
    case 'minimal':
      return hasGame ? `${hook} — ${game}, live on stream.` : `${hook} — caught live on stream.`
    default: return hook
  }
}

function buildInstagramCaption(hook: string, ctx: ClipContext, tone: CopyTone, ctype: ContentType): string {
  const game = ctx.game
  const hasGame = !!game
  const action = describeAction(ctype, ctx).toLowerCase()

  switch (tone) {
    case 'punchy': {
      const flavor = ctype === 'scare'
        ? (hasGame ? `${game} is NOT for the faint-hearted` : 'Horror games are NOT for me')
        : ctype === 'fail'
        ? (hasGame ? `${game} said absolutely not` : 'Why do I even try')
        : (hasGame ? `${action} in ${game}` : 'Stream clips hit different')
      return `${hook}\n\n${flavor}`
    }
    case 'clean': {
      const gameTag = hasGame ? `#${game!.replace(/[^a-zA-Z0-9]/g, '')} ` : ''
      return `${hook}\n\nLive on Twitch — Link in bio\n\n${gameTag}#gaming #twitch`
    }
    case 'funny': {
      const line = ctype === 'fail'
        ? (hasGame ? `${game} really said "absolutely not"` : 'The game said "absolutely not"')
        : (hasGame ? `Streaming ${game} is a lifestyle choice I sometimes regret` : 'Streaming is a lifestyle choice I sometimes regret')
      return `${hook}\n\n${line}\n\n@[channel]`
    }
    case 'hype': {
      const shout = ctype === 'clutch' ? 'BUILT DIFFERENT' : (hasGame ? `${game!.toUpperCase()} GOES CRAZY` : 'WE GO CRAZY ON STREAM')
      return `${hook}\n\n${shout}\n\nFollow for more highlights`
    }
    case 'search': {
      const gameTag = hasGame ? `#${game!.replace(/[^a-zA-Z0-9]/g, '')} ` : ''
      return `${ctx.title}\n\n${hasGame ? game + ' stream highlights' : 'Stream highlights'}\n\n${gameTag}#gaming #gamer #clips #streamer #twitch #reels #viral`
    }
    case 'minimal': return hasGame ? `${hook} — ${game}` : hook
    default: return hook
  }
}


// ── Title generation per platform ──

/** Build a short descriptor for what happened in the clip based on content type and tags. */
function describeAction(ctype: ContentType, ctx: ClipContext): string {
  const tags = [...ctx.eventTags, ...ctx.emotionTags].map(t => t.toLowerCase())
  switch (ctype) {
    case 'clutch': {
      if (tags.some(t => t.includes('1v'))) return tags.find(t => t.includes('1v'))!.toUpperCase() + ' Clutch'
      if (tags.some(t => t.includes('escape'))) return 'Impossible Escape'
      if (tags.some(t => t.includes('save'))) return 'Last-Second Save'
      return 'Insane Clutch'
    }
    case 'fail': {
      if (tags.some(t => t.includes('whiff'))) return 'The Worst Whiff'
      if (tags.some(t => t.includes('rage'))) return 'Rage-Inducing Moment'
      return 'It All Went Wrong'
    }
    case 'scare':
      return tags.some(t => t.includes('jumpscare')) ? 'Jumpscare' : 'This Scared Me'
    case 'reaction':
      return 'My Honest Reaction'
    case 'gameplay': {
      if (tags.some(t => t.includes('kill'))) return 'Clean Kill'
      if (tags.some(t => t.includes('ambush'))) return 'Perfect Ambush'
      if (tags.some(t => t.includes('fight'))) return 'Intense Fight'
      return 'This Play'
    }
    default: return 'This Moment'
  }
}

function generateTitle(ctx: ClipContext, platform: string, tone: CopyTone, ctype: ContentType, hook: string): string {
  const game = ctx.game
  const hasGame = !!game
  const action = describeAction(ctype, ctx)
  const year = new Date().getFullYear()

  if (platform === 'youtube') {
    if (ctype === 'montage') {
      const summary = buildMontageContentSummary(ctx)
      switch (tone) {
        case 'punchy':
          return hasGame ? `Best ${game} Moments That Hit Different` : `Best Stream Moments That Hit Different`
        case 'clean':
          return hasGame
            ? `${game} Stream Highlights — ${ctx.clipCount || ''} Best Clips`.trim()
            : `Stream Highlights — ${ctx.clipCount || ''} Best Clips`.trim()
        case 'funny':
          return hasGame ? `${game} but Everything Goes Wrong` : 'Everything Goes Wrong on Stream'
        case 'hype':
          return hasGame ? `THE MOST INSANE ${game!.toUpperCase()} MONTAGE` : 'THE MOST INSANE STREAM MONTAGE'
        case 'search':
          return hasGame
            ? `${game} Best Moments ${year} — ${summary} | Stream Highlights`
            : `Best Stream Moments ${year} — ${summary} | Highlights`
        case 'minimal':
          return hasGame ? `${game} Highlights` : 'Stream Highlights'
        default:
          return hasGame ? `${game} Highlights` : 'Stream Highlights'
      }
    }

    // Single clip titles — weave game naturally into what happened
    switch (tone) {
      case 'punchy':
        return hasGame ? `${action} in ${game}` : action
      case 'clean':
        return hasGame ? `${ctx.title} | ${game}` : ctx.title
      case 'funny': {
        const funnies = hasGame
          ? [`I Can't Believe This Happened in ${game}`, `${game} Really Said "Not Today"`, `Why Do I Still Play ${game}`]
          : ['I Can\'t Believe This Actually Happened', 'Why Does This Keep Happening to Me', 'I Should Have Stayed in Bed']
        return funnies[Math.floor(Math.random() * funnies.length)]
      }
      case 'hype':
        return hasGame ? `${action} in ${game} — INSANE` : `${action} — INSANE`
      case 'search':
        return hasGame ? `${action} in ${game} | Highlights ${year}` : `${ctx.title} | Highlights ${year}`
      case 'minimal': return ctx.title
      default: return ctx.title
    }
  }

  // TikTok/Instagram — shorter, punchier
  switch (tone) {
    case 'punchy':
      return hasGame ? `${action} in ${game}` : action
    case 'clean':
      return hasGame ? `${ctx.title} | ${game}` : ctx.title
    case 'funny':
      return ctype === 'fail' ? 'pain.' : (hasGame ? `${game} moment` : hook)
    case 'hype':
      return hasGame ? `${action} in ${game}` : action
    case 'search':
      return hasGame ? `${ctx.title} | ${game}` : ctx.title
    case 'minimal': return ctx.title
    default: return hook
  }
}

// ── Hashtag generation (context-aware) ──

/** Shuffle array in-place using Fisher-Yates. Returns the same array. */
function shuffle<T>(arr: T[]): T[] {
  for (let i = arr.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1))
    ;[arr[i], arr[j]] = [arr[j], arr[i]]
  }
  return arr
}

const PLATFORM_TAGS: Record<string, string[]> = {
  tiktok: ['fyp', 'foryou', 'foryoupage', 'viral', 'tiktokgaming', 'tiktok'],
  youtube: ['shorts', 'highlights', 'bestmoments', 'subscribe'],
  instagram: ['reels', 'instareels', 'instagramgaming', 'explore'],
}

/** Platform-required tags — always included first */
const PLATFORM_REQUIRED: Record<string, string[]> = {
  tiktok: ['fyp', 'foryou'],
  youtube: [],    // 'shorts' added only for short-form (≤60s)
  instagram: ['reels'],
}

const CONTENT_TAGS: Record<ContentType, string[]> = {
  reaction: ['reaction', 'screaming', 'jumpscare', 'livereaction', 'streammoment'],
  gameplay: ['gameplay', 'highlights', 'plays', 'epicmoment', 'gamingclips'],
  fail: ['fail', 'funny', 'rage', 'pain', 'epicfail', 'gamerfail'],
  clutch: ['clutch', 'insane', 'goated', 'comeback', 'clutchplay'],
  scare: ['horror', 'jumpscare', 'scary', 'horrorgaming', 'spooked'],
  montage: ['montage', 'compilation', 'bestof', 'bestmoments', 'streamhighlights'],
  generic: ['clips', 'highlights', 'moments', 'gamingclips', 'streamclips'],
}

const MOOD_TAGS: Record<string, string[]> = {
  shock: ['shocked', 'wtf', 'unbelievable', 'noway', 'mindblown'],
  panic: ['panic', 'heartpounding', 'intense', 'tensemoment'],
  hype: ['hype', 'letsgo', 'goated', 'cracked', 'built different'],
  rage: ['rage', 'tilted', 'pain', 'sadge', 'gamerrage'],
  frustration: ['suffering', 'gamerpain', 'why', 'hardstuck'],
  relief: ['relief', 'closecall', 'survived', 'madeIt'],
  surprise: ['surprised', 'unexpected', 'plottwist'],
  fear: ['scared', 'terrified', 'nightmares', 'spooked'],
}

/** Game-specific hashtag variants — main game tag + community abbreviations */
const GAME_VARIANT_TAGS: Record<string, string[]> = {
  'dead by daylight': ['dbd', 'deadbydaylight', 'dbdclips', 'dbdmoments', 'dbdcommunity', 'dbdgameplay'],
  'valorant': ['valorant', 'valorantclips', 'valorantmoments', 'valo', 'valoclips', 'valorantgameplay', 'valoranthighlights'],
  'fortnite': ['fortnite', 'fortniteclips', 'fortnitemoments', 'fortnitebr', 'fortnitegameplay', 'fortnitehighlights'],
  'apex legends': ['apexlegends', 'apex', 'apexclips', 'apexmoments', 'apexgameplay', 'apexhighlights'],
  'overwatch': ['overwatch', 'ow2', 'overwatchclips', 'overwatch2', 'owclips', 'overwatchmoments'],
  'league of legends': ['leagueoflegends', 'lol', 'lolclips', 'leagueclips', 'lolmoments', 'lolgameplay'],
  'minecraft': ['minecraft', 'minecraftclips', 'mcclips', 'minecraftgaming', 'minecraftmoments'],
  'phasmophobia': ['phasmophobia', 'phasmo', 'phasmoclips', 'phasmomoments', 'phasmogameplay'],
  'call of duty': ['callofduty', 'cod', 'codclips', 'warzone', 'codmoments', 'codgameplay'],
  'escape from tarkov': ['tarkov', 'escapefromtarkov', 'eft', 'tarkovclips', 'tarkovmoments'],
  'counter-strike': ['cs2', 'csgo', 'counterstrike', 'csclips', 'cs2clips', 'cs2moments'],
  'gta': ['gta', 'gta5', 'gtaonline', 'gtaclips', 'gtamoments', 'gtagameplay'],
  'elden ring': ['eldenring', 'eldenringclips', 'eldenringmoments', 'eldenringgameplay'],
  'rust': ['rustgame', 'rust', 'rustclips', 'rustmoments', 'playrust'],
  'rainbow six': ['r6', 'rainbowsixsiege', 'r6siege', 'r6clips', 'siegeclips', 'siegemoments'],
  'rocket league': ['rocketleague', 'rl', 'rlclips', 'rocketleagueclips', 'rlmoments'],
  'lethal company': ['lethalcompany', 'lethalcompanyclips', 'lethalcompanymoments'],
  'among us': ['amongus', 'amongusclips', 'amongusmoments', 'amogus'],
  'fall guys': ['fallguys', 'fallguysclips', 'fallguysmoments'],
  'sea of thieves': ['seaofthieves', 'sot', 'sotclips', 'sotmoments'],
  'the finals': ['thefinals', 'thefinalsclips', 'thefinalsmoments'],
  'helldivers': ['helldivers2', 'helldivers', 'helldiversclips', 'helldiversmoments'],
  'palworld': ['palworld', 'palworldclips', 'palworldmoments'],
}

/** Words commonly found in transcripts that suggest content themes */
const TRANSCRIPT_KEYWORD_TAGS: Record<string, string[]> = {
  // Reactions
  'oh my god': ['omg', 'shocked'],
  'no way': ['noway', 'unbelievable'],
  'what the': ['wtf', 'shocked'],
  'let\'s go': ['letsgo', 'hype'],
  'are you kidding': ['noway', 'rage'],
  // Gaming actions
  'one shot': ['oneshot', 'insane'],
  'head shot': ['headshot', 'aimbot'],
  'clutch': ['clutch', 'clutchplay'],
  'squad wipe': ['squadwipe', 'cracked'],
  'team wipe': ['teamwipe', 'ace'],
  'ace': ['ace', 'cracked'],
  'gg': ['gg', 'gaming'],
  'run': ['intense', 'chase'],
  'help': ['panic', 'funny'],
  'behind': ['jumpscare', 'ambush'],
}

/** Game-specific keyword clusters for transcript/subtitle inference.
 *  Each entry needs 2+ keyword matches to count (avoids false positives from common words). */
const KEYWORD_CLUSTERS: [string, string[]][] = [
  ['dead by daylight', ['killer', 'survivor', 'hook', 'generator', 'pallet', 'basement', 'hatch', 'mori', 'entity', 'bloodweb', 'hex', 'totem', 'locker', 'heartbeat']],
  ['valorant', ['spike', 'defuse', 'vandal', 'phantom', 'sage', 'jett', 'omen', 'reyna', 'sova', 'breach', 'chamber', 'raze', 'killjoy', 'cypher', 'astra', 'neon', 'brimstone', 'viper', 'ascent', 'bind', 'haven', 'split', 'icebox', 'fracture', 'pearl', 'lotus']],
  ['fortnite', ['storm', 'llama', 'tilted', 'build', 'shield potion', 'battle bus', 'victory royale', 'slurp', 'chug', 'mats', 'cranking']],
  ['apex legends', ['ring', 'banner', 'respawn beacon', 'wraith', 'pathfinder', 'bloodhound', 'lifeline', 'caustic', 'octane', 'mirage', 'gibraltar', 'wattson', 'crypto', 'revenant', 'loba', 'rampart', 'valkyrie', 'seer', 'ash', 'catalyst']],
  ['overwatch', ['payload', 'point', 'mercy', 'genji', 'tracer', 'reinhardt', 'dva', 'lucio', 'ana', 'winston', 'zarya', 'moira', 'sigma', 'kiriko']],
  ['league of legends', ['turret', 'nexus', 'baron', 'dragon', 'jungle', 'minion', 'lane', 'gank', 'flash', 'ignite', 'tower dive', 'inhibitor', 'rift herald']],
  ['minecraft', ['creeper', 'enderman', 'nether', 'diamond', 'crafting table', 'ender dragon', 'villager', 'pickaxe', 'redstone', 'enchant', 'netherite']],
  ['escape from tarkov', ['extract', 'scav', 'pmc', 'stash', 'flea market', 'hideout', 'labs', 'customs', 'shoreline', 'interchange', 'reserve', 'lighthouse']],
  ['phasmophobia', ['ghost', 'emf', 'crucifix', 'spirit box', 'thermometer', 'sanity', 'hunting', 'freezing', 'fingerprints', 'dots', 'ghost orb']],
  ['call of duty', ['warzone', 'gulag', 'loadout', 'killstreak', 'uav', 'airstrike', 'buy station', 'plate', 'most wanted']],
  ['counter-strike', ['bomb', 'defuse', 'flash', 'smoke', 'awp', 'eco', 'rush b', 'ct side', 't side', 'deagle', 'ak', 'm4']],
  ['among us', ['impostor', 'crewmate', 'vent', 'emergency meeting', 'sus', 'ejected', 'sabotage', 'electrical']],
  ['rocket league', ['boost', 'aerial', 'save', 'demo', 'flip reset', 'ceiling shot', 'kickoff', 'overtime']],
  ['gta', ['heist', 'wanted level', 'los santos', 'cops', 'oppressor', 'garage', 'mod shop', 'cayo']],
]

/**
 * Infer game from transcript/subtitle text using keyword cluster matching.
 * Strips SRT formatting (timestamps, sequence numbers) before scanning.
 * Returns { game, matchCount } or null if no confident match (needs 2+ keywords).
 */
export function inferGameFromTranscript(rawText: string): { game: string; matchCount: number } | null {
  if (!rawText || rawText.length < 20) {
    console.log('[inferGameFromTranscript] Text too short or empty, length:', rawText?.length ?? 0)
    return null
  }

  // Strip SRT formatting: remove sequence numbers, timestamps like "00:00:01,000 --> 00:00:03,000", blank lines
  const plainText = rawText
    .replace(/^\d+\s*$/gm, '')                              // sequence numbers on their own line
    .replace(/\d{2}:\d{2}:\d{2}[,.]\d{3}\s*-->\s*\d{2}:\d{2}:\d{2}[,.]\d{3}/g, '')  // timestamps
    .replace(/\n{2,}/g, ' ')                                 // collapse blank lines
    .toLowerCase()
    .trim()

  console.log('[inferGameFromTranscript] Plain text length:', plainText.length, '| first 200 chars:', JSON.stringify(plainText.slice(0, 200)))

  let bestGame: string | null = null
  let bestCount = 0
  const allMatches: string[] = []

  for (const [game, keywords] of KEYWORD_CLUSTERS) {
    const matched = keywords.filter(kw => plainText.includes(kw))
    if (matched.length > 0) {
      allMatches.push(`${game}: [${matched.join(', ')}] (${matched.length})`)
    }
    if (matched.length >= 2 && matched.length > bestCount) {
      bestGame = game
      bestCount = matched.length
    }
  }

  console.log('[inferGameFromTranscript] Keyword scan results:', allMatches.length > 0 ? allMatches.join(' | ') : 'no matches')

  if (bestGame) {
    console.log('[inferGameFromTranscript] Winner:', bestGame, `with ${bestCount} keyword matches`)
    return { game: bestGame, matchCount: bestCount }
  }

  return null
}

/**
 * Detect the game being played from all available context.
 * Checks explicit game field, VOD title, clip title, highlight tags, and transcript keywords.
 */
export function detectGame(ctx: ClipContext): string | null {
  // 1. Explicit game field (from user manual input or previous detection)
  if (ctx.game) {
    console.log('[detectGame] Using explicit game field:', ctx.game)
    return ctx.game
  }

  // 2. Search VOD title, clip title, and tags for known game names
  const searchText = [ctx.vodTitle, ctx.title, ...ctx.eventTags].filter(Boolean).join(' ').toLowerCase()

  if (searchText) {
    // Check against known games (longest match first to avoid partial matches like "rust" in "frustration")
    const gameNames = Object.keys(GAME_VARIANT_TAGS).sort((a, b) => b.length - a.length)
    for (const game of gameNames) {
      if (searchText.includes(game)) {
        console.log('[detectGame] Found game name in titles/tags:', game)
        return game
      }
    }

    // Check common abbreviations in title/tags
    const abbrevs: Record<string, string> = {
      dbd: 'dead by daylight', valo: 'valorant', fn: 'fortnite', apex: 'apex legends',
      ow2: 'overwatch', ow: 'overwatch', lol: 'league of legends', mc: 'minecraft',
      eft: 'escape from tarkov', csgo: 'counter-strike', cs2: 'counter-strike',
      r6: 'rainbow six', rl: 'rocket league', sot: 'sea of thieves',
    }
    const words = searchText.split(/\s+/)
    for (const word of words) {
      if (abbrevs[word]) {
        console.log('[detectGame] Matched abbreviation:', word, '→', abbrevs[word])
        return abbrevs[word]
      }
    }
  }

  // 3. Infer from transcript/subtitle content using keyword clusters
  const transcript = ctx.transcript || ctx.transcriptExcerpt || ''
  const result = inferGameFromTranscript(transcript)
  if (result) return result.game

  console.log('[detectGame] No game detected from any source')
  return null
}

/**
 * Extract keywords from transcript text that map to relevant hashtags.
 * Returns unique tags derived from what the streamer actually said.
 */
function extractTranscriptTags(transcript: string | undefined): string[] {
  if (!transcript || transcript.length < 10) return []
  const lower = transcript.toLowerCase()
  const tags: string[] = []
  const seen = new Set<string>()

  for (const [phrase, mappedTags] of Object.entries(TRANSCRIPT_KEYWORD_TAGS)) {
    if (lower.includes(phrase)) {
      for (const t of mappedTags) {
        if (!seen.has(t)) { seen.add(t); tags.push(t) }
      }
    }
  }
  return tags
}

/**
 * Context-aware hashtag generation.
 * Produces a unique set each call (randomized selection from ranked pools).
 */
function generateHashtags(ctx: ClipContext, platform: string, ctype: ContentType, count: number): string[] {
  const tags = new Set<string>()

  // 1. Platform-required tags always first
  const required = [...(PLATFORM_REQUIRED[platform] || [])]
  if (platform === 'youtube' && ctx.duration <= 60) required.push('shorts')
  for (const t of required) tags.add(t)

  // 2. Game tags — highest priority
  const game = detectGame(ctx)
  if (game) {
    const gameLower = game.toLowerCase()
    const gameTag = gameLower.replace(/[^a-z0-9]/g, '')
    tags.add(gameTag)
    console.log('[generateHashtags] Game detected:', game, '→ base tag:', gameTag)

    // Try to match known game variant tags
    let matched = false
    for (const [key, variants] of Object.entries(GAME_VARIANT_TAGS)) {
      if (gameLower.includes(key) || key.includes(gameLower)) {
        const shuffled = shuffle([...variants.filter(v => v !== gameTag)])
        for (const t of shuffled.slice(0, 2)) { if (tags.size < count) tags.add(t) }
        console.log('[generateHashtags] Matched GAME_VARIANT_TAGS key:', key, '— added variant tags')
        matched = true
        break
      }
    }

    // Fallback for unknown games: add generic game+clips/moments tags
    if (!matched) {
      console.log('[generateHashtags] No variant match for', game, '— generating fallback tags')
      if (tags.size < count) tags.add(`${gameTag}clips`)
      if (tags.size < count) tags.add(`${gameTag}moments`)
      if (tags.size < count) tags.add(`${gameTag}gameplay`)
    }
  } else {
    console.log('[generateHashtags] No game detected — skipping game tags')
  }

  // 3. Content-type tags (shuffled for variety)
  const contentPool = shuffle([...(CONTENT_TAGS[ctype] || [])])
  for (const t of contentPool.slice(0, 2)) { if (tags.size < count) tags.add(t) }

  // 4. Transcript-derived tags
  const transcriptTags = shuffle(extractTranscriptTags(ctx.transcript || ctx.transcriptExcerpt))
  for (const t of transcriptTags.slice(0, 2)) { if (tags.size < count) tags.add(t) }

  // 5. Emotion/mood tags
  for (const emo of ctx.emotionTags) {
    const pool = shuffle([...(MOOD_TAGS[emo.toLowerCase()] || [])])
    for (const t of pool.slice(0, 1)) { if (tags.size < count) tags.add(t) }
  }

  // 6. Platform discovery tags (shuffled)
  const platformPool = shuffle([...(PLATFORM_TAGS[platform] || [])]).filter(t => !tags.has(t))
  for (const t of platformPool.slice(0, 1)) { if (tags.size < count) tags.add(t) }

  // 7. Generic fill only if under budget
  const genericPool = shuffle(['gaming', 'gamer', 'twitch', 'streamer', 'twitchstreamer', 'gamingcommunity', 'twitchclips'])
  for (const t of genericPool) { if (tags.size < count) tags.add(t) }

  return [...tags].slice(0, count)
}

// ── Expanded hashtag suggestions (for the suggestion panel) ──

/** Generate a large pool of ranked hashtag suggestions. User picks from these. */
export function generateHashtagSuggestions(
  ctx: ClipContext,
  platform: string,
): { tag: string; category: string; relevance: number }[] {
  const ctype = classifyContent(ctx)
  const suggestions: { tag: string; category: string; relevance: number }[] = []
  const seen = new Set<string>()

  const add = (tag: string, category: string, relevance: number) => {
    const clean = tag.toLowerCase().replace(/[^a-z0-9]/g, '')
    if (!clean || seen.has(clean)) return
    seen.add(clean)
    suggestions.push({ tag: clean, category, relevance })
  }

  // Game — highest relevance (with auto-detection)
  const game = detectGame(ctx)
  if (game) {
    const gameTag = game.toLowerCase().replace(/[^a-z0-9]/g, '')
    add(gameTag, 'game', 1.0)
    console.log('[generateHashtagSuggestions] Game:', game, '→ tag:', gameTag)

    let matched = false
    const gameKey = game.toLowerCase()
    for (const [key, variantTags] of Object.entries(GAME_VARIANT_TAGS)) {
      if (gameKey.includes(key) || key.includes(gameKey)) {
        for (const t of variantTags) add(t, 'game', 0.95)
        matched = true
      }
    }

    // Fallback for unknown games
    if (!matched) {
      console.log('[generateHashtagSuggestions] Unknown game, generating fallback tags for:', game)
      add(`${gameTag}clips`, 'game', 0.93)
      add(`${gameTag}moments`, 'game', 0.92)
      add(`${gameTag}gameplay`, 'game', 0.91)
      add(`${gameTag}highlights`, 'game', 0.90)
    }
  } else {
    console.log('[generateHashtagSuggestions] No game detected')
  }

  // Transcript-derived (high relevance — contextual)
  const transcriptTags = extractTranscriptTags(ctx.transcript || ctx.transcriptExcerpt)
  for (const t of transcriptTags) add(t, 'transcript', 0.88)

  // Content type
  for (const t of CONTENT_TAGS[ctype] || []) add(t, 'content', 0.85)

  // Mood/emotion
  for (const emo of ctx.emotionTags) {
    for (const t of MOOD_TAGS[emo.toLowerCase()] || []) add(t, 'mood', 0.75)
  }

  // Event tags as hashtags
  for (const ev of ctx.eventTags.slice(0, 5)) add(ev, 'event', 0.7)

  // Platform-required (high priority in suggestions too)
  const required = PLATFORM_REQUIRED[platform] || []
  for (const t of required) add(t, 'platform', 0.92)
  if (platform === 'youtube' && ctx.duration <= 60) add('shorts', 'platform', 0.92)

  // Platform discovery
  for (const t of PLATFORM_TAGS[platform] || []) add(t, 'platform', 0.65)

  // Generic gaming/streaming
  for (const t of ['gaming', 'gamer', 'twitch', 'streamer', 'twitchstreamer', 'gamingcommunity', 'twitchclips', 'streamhighlights']) {
    add(t, 'general', 0.5)
  }

  // Trending/viral
  for (const t of ['viral', 'trending', 'foryou', 'explore', 'blowup']) add(t, 'discovery', 0.4)

  // Montage-specific
  if (ctx.isMontage) {
    for (const t of ['montage', 'compilation', 'bestof', 'highlights', 'bestmoments', 'streamhighlights']) add(t, 'format', 0.8)
  }

  // Sort by relevance
  suggestions.sort((a, b) => b.relevance - a.relevance)
  return suggestions
}

// ── Transcript-aware hook generation ──

/** Extract a SHORT usable phrase (max ~8 words) from the transcript for captions.
 *  Never returns the full transcript — cherry-picks the best expressive snippet. */
function extractTranscriptHook(transcript: string | undefined, offset = 0): string | null {
  if (!transcript || transcript.trim().length < 5) return null

  // Normalize: lowercase, collapse whitespace
  const clean = transcript.toLowerCase().replace(/\s+/g, ' ').trim()

  // Split aggressively — punctuation, conjunctions, fillers, pauses
  const segments = clean
    .split(/[.!?,;:]+|\b(?:and then|but then|so then|and|but|so|like|okay|alright|anyway|um|uh|i mean|you know|i think)\b/i)
    .map(s => s.trim())
    .filter(s => s.length >= 3)

  const MAX_WORDS = 8
  const MIN_WORDS = 2
  const candidates: string[] = []
  const seen = new Set<string>()

  for (const seg of segments) {
    const words = seg.split(/\s+/)
    if (words.length >= MIN_WORDS && words.length <= MAX_WORDS) {
      const phrase = words.join(' ')
      if (!seen.has(phrase)) { seen.add(phrase); candidates.push(phrase) }
    } else if (words.length > MAX_WORDS) {
      // Take just the first chunk and last chunk — no sliding window
      const first = words.slice(0, MAX_WORDS).join(' ')
      const last = words.slice(-MAX_WORDS).join(' ')
      if (!seen.has(first)) { seen.add(first); candidates.push(first) }
      if (first !== last && !seen.has(last)) { seen.add(last); candidates.push(last) }
    }
  }

  if (candidates.length === 0) return null

  // Score: prefer short, expressive, complete-feeling phrases
  const EXPRESSIVE = /\b(no|nah|oh|wait|what|why|how|dude|bro|yo|bruh|omg|god|damn|help|run|go|stop|dead|kill|clutch|nice|insane|crazy|pity|let's go|potato|bot|trash|gg|oof|rip|nope|yep|hell|scared|die|dying|get me|save me)\b/i
  const scored = candidates.map(phrase => {
    const wc = phrase.split(/\s+/).length
    return {
      phrase,
      score:
        (EXPRESSIVE.test(phrase) ? 10 : 0) +    // expressive words
        (wc >= 3 && wc <= 6 ? 5 : 0) +          // sweet spot length
        (wc <= 4 ? 3 : 0) +                      // brevity bonus
        (phrase.length <= 30 ? 2 : 0),            // short char count
    }
  })
  scored.sort((a, b) => b.score - a.score)

  // Deduplicate top results (no near-duplicates that share 80%+ words)
  const unique: typeof scored = []
  for (const entry of scored) {
    const entryWords = new Set(entry.phrase.split(/\s+/))
    const isDupe = unique.some(u => {
      const uWords = new Set(u.phrase.split(/\s+/))
      const overlap = [...entryWords].filter(w => uWords.has(w)).length
      return overlap / Math.max(entryWords.size, uWords.size) > 0.7
    })
    if (!isDupe) unique.push(entry)
  }

  const pool = unique.length > 0 ? unique : scored
  const top = pool.slice(0, Math.max(3, Math.ceil(pool.length / 2)))
  return top[offset % top.length].phrase
}

/** Turn a backend event summary into a platform-ready hook line. */
function hookFromSummary(summary: string, ctype: ContentType): string {
  // Capitalize first letter
  const cap = summary.charAt(0).toUpperCase() + summary.slice(1)

  // Content-type-specific framing around the concrete event
  switch (ctype) {
    case 'scare':   return `The moment ${summary}`
    case 'fail':    return cap
    case 'clutch':  return cap
    case 'reaction': return `${cap} and the reaction was instant`
    case 'gameplay': return cap
    case 'montage':  return pickRandom(HOOKS.montage)
    default:         return cap
  }
}

// ── Public API ──

function pickRandom<T>(arr: T[]): T {
  return arr[Math.floor(Math.random() * arr.length)]
}

export function generatePublishCopy(ctx: ClipContext, platform: string, tone: CopyTone, variation = 0): GeneratedCopy {
  console.log('[generatePublishCopy] Input game:', JSON.stringify(ctx.game), '| platform:', platform, '| tone:', tone)
  const ctype = classifyContent(ctx)
  // Auto-detect game from context if not explicitly set
  const detectedGame = detectGame(ctx)
  const enrichedCtx = detectedGame && !ctx.game ? { ...ctx, game: detectedGame } : ctx
  console.log('[generatePublishCopy] Enriched game:', JSON.stringify(enrichedCtx.game))

  // Priority: 1) transcript quote, 2) event summary, 3) title-based, 4) generic pool
  const transcriptHook = extractTranscriptHook(enrichedCtx.transcript || enrichedCtx.transcriptExcerpt, variation)
  let hook: string
  if (enrichedCtx.eventSummary) {
    hook = hookFromSummary(enrichedCtx.eventSummary, ctype)
  } else if (transcriptHook) {
    // Use actual dialogue from the clip
    hook = `"${transcriptHook}"`
  } else if (enrichedCtx.title && enrichedCtx.title.length > 3) {
    hook = enrichedCtx.title
  } else {
    // Use variation to pick different hook from pool
    hook = HOOKS[ctype][variation % HOOKS[ctype].length]
  }

  const captionBuilder = platform === 'youtube' ? buildYouTubeCaption
    : platform === 'instagram' ? buildInstagramCaption
    : (h: string, c: ClipContext, t: CopyTone, ct: ContentType) => buildTikTokCaption(h, c, t, ct, variation)

  const description = captionBuilder(hook, enrichedCtx, tone, ctype)
  const title = generateTitle(enrichedCtx, platform, tone, ctype, hook)

  const hashtagCount = platform === 'youtube' ? 15 : platform === 'instagram' ? 12 : 5
  const hashtags = generateHashtags(enrichedCtx, platform, ctype, hashtagCount)

  return { title, description, hashtags, tone }
}

export function generateAllVariants(ctx: ClipContext, platform: string, variation = 0): GeneratedCopy[] {
  const tones: CopyTone[] = ['punchy', 'clean', 'funny', 'hype', 'search', 'minimal']
  return tones.map(tone => generatePublishCopy(ctx, platform, tone, variation))
}

/**
 * Generate a standalone title from scratch.
 * Does NOT use or append to the existing title — produces a completely new one.
 * Each call randomizes from a large pool so repeated presses feel fresh.
 */
export function generateStandaloneTitle(ctx: ClipContext): string {
  const ctype = classifyContent(ctx)
  const pool = ctx.game
    ? TITLE_TEMPLATES[ctype].withGame
    : TITLE_TEMPLATES[ctype].noGame
  const shuffled = shuffle([...pool])
  let title = shuffled[0]
  if (ctx.game) {
    title = title.replace(/\{game\}/g, ctx.game)
  }
  title = title.replace(/\{action\}/g, describeAction(ctype, ctx))

  // If we have a transcript quote, occasionally weave it into the title
  const quote = extractTranscriptHook(ctx.transcript || ctx.transcriptExcerpt, Math.floor(Math.random() * 50))
  if (quote && Math.random() < 0.3 && quote.length <= 40) {
    // ~30% chance to use a quote-based title variant
    const quoteTitles = ctx.game
      ? [`"${quote}" in ${ctx.game}`, `"${quote}" — ${ctx.game} Moment`]
      : [`"${quote}"`, `"${quote}" — Stream Moment`]
    title = pickRandom(quoteTitles)
  }

  return title
}

/**
 * Generate a standalone caption from the template pool.
 * Uses game, transcript, content type, and tone to produce a platform-ready caption.
 * No hashtags included. Each call randomizes from the pool.
 */
export function generateStandaloneCaption(ctx: ClipContext, tone: CopyTone): string {
  const ctype = classifyContent(ctx)
  const action = describeAction(ctype, ctx)
  const quote = extractTranscriptHook(ctx.transcript || ctx.transcriptExcerpt, Math.floor(Math.random() * 50))

  // Look up tone pool — fall back to 'punchy' if unknown
  const pool = CAPTION_POOLS[tone]
  if (!pool || pool.length === 0) {
    console.warn(`[CaptionGen] No pool for tone "${tone}", falling back to punchy`)
    const fallback = CAPTION_POOLS.punchy
    if (!fallback || fallback.length === 0) return ctx.title || ''
  }
  const activePool = pool && pool.length > 0 ? pool : CAPTION_POOLS.punchy

  const shuffled = shuffle([...activePool])
  const tctx = { game: ctx.game, quote: quote || undefined, action, ctype, title: ctx.title || undefined }
  // Try all templates in shuffled order — return first non-empty result
  for (let i = 0; i < shuffled.length; i++) {
    const result = shuffled[i](tctx)
    if (result && result.trim().length > 0) {
      console.log(`[CaptionGen] tone="${tone}" pool_size=${activePool.length} picked_index=${i} result="${result.trim().slice(0, 60)}..."`)
      return result.trim()
    }
  }
  // Absolute fallback — shouldn't happen if pools have no-quote entries
  console.warn(`[CaptionGen] All ${activePool.length} templates for tone "${tone}" returned empty`)
  return ctx.title || action || 'Check this out'
}

export const TONE_LABELS: Record<CopyTone, { label: string; emoji: string }> = {
  // Frontend generator tones
  punchy: { label: 'Punchy', emoji: '💥' },
  clean: { label: 'Clean', emoji: '✨' },
  funny: { label: 'Funny', emoji: '😂' },
  hype: { label: 'Hype', emoji: '🔥' },
  search: { label: 'SEO', emoji: '🔍' },
  minimal: { label: 'Minimal', emoji: '—' },
  // Backend mode labels
  direct_quote: { label: 'Quote', emoji: '"' },
  blame: { label: 'Blame', emoji: '👉' },
  internal_thought: { label: 'Thought', emoji: '💭' },
  observation: { label: 'Observe', emoji: '👁' },
}
