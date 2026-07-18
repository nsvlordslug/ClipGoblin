import test from 'node:test'
import assert from 'node:assert/strict'
import { existsSync, readFileSync } from 'node:fs'
import { CAPTION_STYLES } from '../src/lib/editTypes.ts'
import { EMPHASIS_STYLES } from '../src/lib/captionEmphasis.ts'
import {
  clampCaptionFontScale,
  fitCaptionFontSize,
  longestCaptionWordLength,
} from '../src/lib/captionSizing.ts'

test('Cardboard and Highlight are visually distinct, readable presets', () => {
  const cardboard = CAPTION_STYLES.find(style => style.id === 'bold-white')
  const highlight = CAPTION_STYLES.find(style => style.id === 'fire')

  assert.equal(cardboard?.name, 'Cardboard')
  assert.equal(cardboard?.presentation, 'cardboard')
  assert.equal(cardboard?.fontColor, '#7A2118')
  assert.equal(cardboard?.bgColor, '#C99358')
  assert.equal(cardboard?.uppercase, true)

  assert.equal(highlight?.name, 'Highlight')
  assert.match(highlight?.fontFamily || '', /Rubik Dirt/)
  assert.equal(highlight?.uppercase, true)
  assert.ok((highlight?.strokeWidth || 0) >= 3)
})

test('Highlight ships its redistributable font and OFL license', () => {
  const fontPath = new URL('../public/fonts/RubikDirt-Regular.ttf', import.meta.url)
  const licensePath = new URL('../public/fonts/OFL-RubikDirt.txt', import.meta.url)

  assert.equal(existsSync(fontPath), true)
  assert.equal(existsSync(licensePath), true)
  assert.match(readFileSync(licensePath, 'utf8'), /SIL OPEN FONT LICENSE Version 1\.1/)
})

test('Frosted, Drip, and Comic Pop replace the plain presets with bundled OFL fonts', () => {
  const expected = [
    { id: 'boxed', name: 'Frosted', family: 'Coiny', file: 'Coiny-Regular.ttf', license: 'OFL-Coiny.txt' },
    { id: 'minimal', name: 'Drip', family: 'Nosifer', file: 'Nosifer-Regular.ttf', license: 'OFL-Nosifer.txt' },
    { id: 'comic-pop', name: 'Comic Pop', family: 'Bangers', file: 'Bangers-Regular.ttf', license: 'OFL-Bangers.txt' },
  ]

  for (const item of expected) {
    const style = CAPTION_STYLES.find(candidate => candidate.id === item.id)
    assert.equal(style?.name, item.name)
    assert.match(style?.fontFamily || '', new RegExp(item.family))
    assert.equal(existsSync(new URL(`../public/fonts/${item.file}`, import.meta.url)), true)
    const license = readFileSync(new URL(`../public/fonts/${item.license}`, import.meta.url), 'utf8')
    assert.match(license, /SIL OPEN FONT LICENSE Version 1\.1/)
  }

  const frosted = CAPTION_STYLES.find(style => style.id === 'boxed')
  assert.equal(frosted?.fontColor, '#FFFFFF')
  assert.equal(EMPHASIS_STYLES.boxed.color, '#FF8FD8')
})

test('Tape Riot, Paper Mischief, and Goblin Bite ship custom faces and material fonts', () => {
  const expected = [
    {
      id: 'tape-riot', name: 'Tape Riot', presentation: 'tape-riot',
      family: 'ClipGoblin Tape Riot', license: 'OFL-RussoOne.txt',
      files: [
        'ClipGoblinTapeRiot-Regular.ttf',
        'ClipGoblinTapeRiotSeams-Regular.ttf',
        'ClipGoblinTapeRiotPatches-Regular.ttf',
      ],
      face: '#B8FF2C', emphasis: '#A855F7',
    },
    {
      id: 'paper-mischief', name: 'Paper Mischief', presentation: 'paper-mischief',
      family: 'ClipGoblin Paper Mischief', license: 'OFL-TitanOne.txt',
      files: [
        'ClipGoblinPaperMischief-Regular.ttf',
        'ClipGoblinPaperMischiefFiber-Regular.ttf',
        'ClipGoblinPaperMischiefTabs-Regular.ttf',
      ],
      face: '#F3F0E8', emphasis: '#B8FF2C',
    },
    {
      id: 'goblin-bite', name: 'Goblin Bite', presentation: 'goblin-bite',
      family: 'ClipGoblin Goblin Bite', license: 'OFL-Anton.txt',
      files: [
        'ClipGoblinGoblinBite-Regular.ttf',
        'ClipGoblinGoblinBiteDistress-Regular.ttf',
      ],
      face: '#DFFF20', emphasis: '#FFFFFF',
    },
  ]

  for (const item of expected) {
    const style = CAPTION_STYLES.find(candidate => candidate.id === item.id)
    assert.equal(style?.name, item.name)
    assert.equal(style?.presentation, item.presentation)
    assert.match(style?.fontFamily || '', new RegExp(item.family))
    assert.equal(style?.fontColor, item.face)
    assert.equal(style?.shadow, 'none')
    assert.equal(style?.uppercase, true)
    assert.ok((style?.safeWidthRatio || 1) <= 0.8)
    assert.equal(EMPHASIS_STYLES[item.id].color, item.emphasis)
    for (const file of item.files) {
      assert.equal(existsSync(new URL(`../public/fonts/${file}`, import.meta.url)), true)
    }
    const license = readFileSync(new URL(`../public/fonts/${item.license}`, import.meta.url), 'utf8')
    assert.match(license, /SIL OPEN FONT LICENSE Version 1\.1/)
  }

  const generator = readFileSync(new URL('../tools/generate_caption_fonts.py', import.meta.url), 'utf8')
  assert.match(generator, /tape_cutouts/)
  assert.match(generator, /paper_cutouts/)
  assert.match(generator, /goblin_cutouts/)

  assert.equal(new Set(expected.map(item => item.face)).size, expected.length)
  assert.equal(new Set(expected.map(item => item.emphasis)).size, expected.length)
  assert.notEqual(
    CAPTION_STYLES.find(style => style.id === 'tape-riot')?.fontFamily,
    CAPTION_STYLES.find(style => style.id === 'comic-pop')?.fontFamily,
  )
  assert.notEqual(
    CAPTION_STYLES.find(style => style.id === 'paper-mischief')?.fontFamily,
    CAPTION_STYLES.find(style => style.id === 'boxed')?.fontFamily,
  )
  assert.notEqual(
    CAPTION_STYLES.find(style => style.id === 'goblin-bite')?.fontFamily,
    CAPTION_STYLES.find(style => style.id === 'minimal')?.fontFamily,
  )
})

test('caption sizing clamps user scale and shrinks long words into a vertical safe area', () => {
  assert.equal(clampCaptionFontScale(0.2), 0.75)
  assert.equal(clampCaptionFontScale(4), 1.25)
  assert.equal(clampCaptionFontScale(Number.NaN), 1)
  assert.equal(longestCaptionWordLength('short extraordinarilylongword okay'), 23)

  const normal = fitCaptionFontSize({
    requestedPx: 24,
    frameWidth: 270,
    isVertical: true,
    text: 'clutch',
    characterWidthFactor: 0.7,
  })
  const long = fitCaptionFontSize({
    requestedPx: 24,
    frameWidth: 270,
    isVertical: true,
    text: 'extraordinarilylongword',
    characterWidthFactor: 0.7,
  })

  assert.ok(Math.abs(normal - 22.95) < 0.0001)
  assert.ok(long < normal)
  assert.ok(long * 23 * 0.7 <= 270 * 0.84 + 0.001)
})
