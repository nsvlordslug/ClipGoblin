import test from 'node:test'
import assert from 'node:assert/strict'
import { existsSync, readFileSync } from 'node:fs'
import { CAPTION_STYLES } from '../src/lib/editTypes.ts'
import { EMPHASIS_STYLES } from '../src/lib/captionEmphasis.ts'
import {
  clampCaptionCardScale,
  clampCaptionFontScale,
  DEFAULT_CAPTION_CARD_SCALE,
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
  assert.equal(
    existsSync(new URL('../public/caption-materials/cardboard-placard-v1.png', import.meta.url)),
    true,
  )
  const cardboardRenderer = readFileSync(
    new URL('../src-tauri/src/cardboard_caption.rs', import.meta.url),
    'utf8',
  )
  assert.match(cardboardRenderer, /cardboard-reference-v6/)
  assert.match(cardboardRenderer, /placard_geometry_is_stable_across_caption_changes/)

  const editor = readFileSync(new URL('../src/pages/Editor.tsx', import.meta.url), 'utf8')
  assert.match(editor, /captionStyle\.presentation === 'cardboard'/)
  assert.match(editor, />Card Size</)
  assert.match(editor, /cardScale=\{captionCardScale\}/)

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

test('Frosted, Glossy Thumbnail, and Comic Pop replace the plain presets with bundled OFL fonts', () => {
  const expected = [
    { id: 'boxed', name: 'Frosted', family: 'Coiny', file: 'Coiny-Regular.ttf', license: 'OFL-Coiny.txt' },
    { id: 'minimal', name: 'Glossy Thumbnail', family: 'Anton', file: 'Anton-Regular.ttf', license: 'OFL-Anton.txt' },
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
    CAPTION_STYLES.find(style => style.id === 'goblin-bite')?.presentation,
    CAPTION_STYLES.find(style => style.id === 'minimal')?.presentation,
  )

  const paper = CAPTION_STYLES.find(style => style.id === 'paper-mischief')
  assert.equal(paper?.strokeWidth, 0)
  assert.equal(paper?.strokeColor, '')

  const frosted = CAPTION_STYLES.find(style => style.id === 'boxed')
  assert.equal(frosted?.strokeWidth, 3)
  assert.equal(frosted?.strokeColor, '#FFFFFF')
})

test('Glossy Thumbnail replaces the existing Drip slot with one native preview/export style', () => {
  const glossySlots = CAPTION_STYLES.filter(style => style.id === 'minimal')
  assert.equal(glossySlots.length, 1)
  assert.equal(CAPTION_STYLES.some(style => style.name === 'Drip'), false)
  assert.equal(glossySlots[0]?.name, 'Glossy Thumbnail')
  assert.equal(glossySlots[0]?.presentation, 'glossy-thumbnail')
  assert.equal(glossySlots[0]?.uppercase, true)
  assert.equal(glossySlots[0]?.shadow, 'none')
  assert.ok((glossySlots[0]?.safeWidthRatio || 1) <= 0.72)

  const root = '../public/caption-glyphs/glossy-thumbnail'
  const metadata = JSON.parse(readFileSync(new URL(`${root}/metadata.json`, import.meta.url), 'utf8'))
  assert.equal(existsSync(new URL(`${root}/atlas.png`, import.meta.url)), true)
  assert.equal(existsSync(new URL(`${root}/picker.png`, import.meta.url)), true)
  assert.equal(
    existsSync(new URL('../public/caption-materials/glossy-thumbnail-burst-v1.png', import.meta.url)),
    true,
  )
  assert.equal(metadata.rendererVersion, 'glossy-thumbnail-image-glyph-v1')
  assert.equal(metadata.styleId, 'minimal')
  assert.equal(metadata.displayName, 'Glossy Thumbnail')
  assert.equal(metadata.textTransform, 'uppercase')
  assert.equal(metadata.referencePixelsShipped, false)
  assert.equal(metadata.sourceFont, 'Anton Regular')
  assert.match(metadata.sourceFontLicense, /SIL Open Font License 1\.1/)
  assert.match(metadata.material.face, /yellow-to-orange glossy gradient/i)
  assert.match(metadata.material.edge, /white keyline/i)
  assert.match(metadata.material.depth, /gold.*extrusion/i)
  assert.match(metadata.material.card, /purple radial burst/i)

  for (const character of 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!"#$%&\'()*+,-./:;<=>?@[\\]^_`{|}~') {
    const entry = metadata.glyphs[character]
    assert.ok(Array.isArray(entry?.atlas) || typeof entry?.alias === 'string', `missing Glossy glyph ${character}`)
  }

  const preview = readFileSync(new URL('../src/components/CaptionPreview.tsx', import.meta.url), 'utf8')
  const player = readFileSync(new URL('../src/components/ClipPlayer.tsx', import.meta.url), 'utf8')
  const editor = readFileSync(new URL('../src/pages/Editor.tsx', import.meta.url), 'utf8')
  const renderer = readFileSync(new URL('../src-tauri/src/image_glyph_caption.rs', import.meta.url), 'utf8')
  assert.match(preview, /cs\.presentation === 'glossy-thumbnail'/)
  assert.match(preview, /styleId: cs\.id/)
  assert.match(preview, /GLOSSY_PRELOAD_CONCURRENCY/)
  assert.match(preview, /decodeCaptionImage/)
  assert.match(preview, /glossyDecodedImagesRef/)
  assert.match(player, /playbackBlocked/)
  assert.match(editor, /captionProvenance=\{captionProvenance\}/)
  assert.match(editor, /playbackBlocked=\{captionPreviewPreparing\}/)
  assert.match(editor, /caption-glyphs\/\$\{s\.presentation\}\/picker\.png/)
  assert.match(renderer, /GLOSSY_THUMBNAIL_RENDERER_VERSION/)
  assert.match(renderer, /"minimal" => Some\(PackSource/)
})

test('Undead Legion ships a complete original reusable image-glyph pack', () => {
  const style = CAPTION_STYLES.find(candidate => candidate.id === 'undead-legion')
  assert.equal(style?.name, 'Undead Legion')
  assert.equal(style?.presentation, 'undead-legion')
  assert.equal(style?.fontColor, '#B2FF1C')
  assert.equal(style?.uppercase, false)
  assert.equal(style?.shadow, 'none')
  assert.ok((style?.safeWidthRatio || 1) <= 0.8)
  assert.equal(EMPHASIS_STYLES['undead-legion'].color, '#FF30CD')

  const atlasPath = new URL('../public/caption-glyphs/undead-legion/atlas.png', import.meta.url)
  const metadataPath = new URL('../public/caption-glyphs/undead-legion/metadata.json', import.meta.url)
  assert.equal(existsSync(atlasPath), true)
  assert.equal(existsSync(metadataPath), true)

  const metadata = JSON.parse(readFileSync(metadataPath, 'utf8'))
  assert.equal(metadata.rendererVersion, 'undead-legion-image-glyph-v4')
  assert.equal(metadata.originalMaterialArtwork, true)
  assert.match(metadata.material.face, /lower-face magenta paint/i)
  assert.equal(metadata.sourceFont, null)
  assert.match(metadata.sourceFontLicense, /no external font skeleton/i)
  assert.match(metadata.construction, /cleaned transparent image glyphs/i)
  assert.match(metadata.sourceSheetSha256, /^[a-f0-9]{64}$/)
  assert.equal(metadata.coverage.physicalGlyphs, 94)
  assert.equal(metadata.coverage.primarySheetCleaned, 80)
  assert.equal(metadata.coverage.authoredSymbolFallback, 14)

  const required = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!"#$%&\'()*+,-./:;<=>?@[\\]^_`{|}~'
  for (const character of required) {
    const entry = metadata.glyphs[character]
    assert.ok(Array.isArray(entry?.atlas), `missing atlas metadata for ${character}`)
  }

  for (const character of 'abcdefghijklmnopqrstuvwxyz') {
    assert.ok(Array.isArray(metadata.glyphs[character]?.atlas), `missing physical lowercase glyph for ${character}`)
    assert.equal(metadata.glyphs[character]?.alias, undefined)
    assert.equal(metadata.glyphs[character]?.sourceKind, 'primary-sheet-cleaned')
  }

  assert.equal(metadata.glyphs.A.sourceKind, 'primary-sheet-cleaned')
  assert.equal(metadata.glyphs['&'].sourceKind, 'authored-symbol-fallback')

  const renderer = readFileSync(new URL('../src-tauri/src/undead_legion.rs', import.meta.url), 'utf8')
  assert.match(renderer, /include_bytes!\("\.\.\/\.\.\/public\/caption-glyphs\/undead-legion\/atlas\.png"\)/)
  assert.match(renderer, /include_str!\("\.\.\/\.\.\/public\/caption-glyphs\/undead-legion\/metadata\.json"\)/)
})

test('Hellfire uses its cleaned image-glyph atlas in place of a generic font skeleton', () => {
  const style = CAPTION_STYLES.find(candidate => candidate.id === 'hellfire')
  assert.equal(style?.name, 'Hellfire')
  assert.equal(style?.presentation, 'hellfire')
  assert.equal(style?.shadow, 'none')
  assert.ok((style?.safeWidthRatio || 1) <= 0.8)

  const root = '../public/caption-glyphs/hellfire'
  const metadataPath = new URL(`${root}/metadata.json`, import.meta.url)
  assert.equal(existsSync(new URL(`${root}/atlas.png`, import.meta.url)), true)
  assert.equal(existsSync(new URL(`${root}/picker.png`, import.meta.url)), true)
  const metadata = JSON.parse(readFileSync(metadataPath, 'utf8'))
  assert.equal(metadata.rendererVersion, 'hellfire-image-glyph-v3')
  assert.equal(metadata.styleId, 'hellfire')
  assert.equal(metadata.sourceStored, false)
  assert.match(metadata.construction, /no generic font skeleton/i)
  assert.match(metadata.material.face, /weathered silver metal/i)
  assert.match(metadata.material.depth, /deep red source sidewall/i)

  const required = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!"#$%&\'()*+,-./:;<=>?@[\\]^_`{|}~'
  for (const character of required) {
    assert.ok(Array.isArray(metadata.glyphs[character]?.atlas), `missing Hellfire atlas entry for ${character}`)
  }

  assert.equal(metadata.glyphs.A.sourceKind, 'primary-sheet-cleaned')
  assert.equal(metadata.glyphs.n.sourceKind, 'source-derived-case-fallback')
  assert.equal(metadata.glyphs._.sourceKind, 'authored-symbol-fallback')
})

test('Horror preserves the sheet disintegration material in a reusable image-glyph pack', () => {
  const style = CAPTION_STYLES.find(candidate => candidate.id === 'horror')
  assert.equal(style?.name, 'Horror')
  assert.equal(style?.presentation, 'horror')
  assert.equal(style?.shadow, 'none')
  assert.ok((style?.safeWidthRatio || 1) <= 0.8)

  const root = '../public/caption-glyphs/horror'
  const metadata = JSON.parse(readFileSync(new URL(`${root}/metadata.json`, import.meta.url), 'utf8'))
  assert.equal(existsSync(new URL(`${root}/atlas.png`, import.meta.url)), true)
  assert.equal(existsSync(new URL(`${root}/picker.png`, import.meta.url)), true)
  assert.equal(metadata.rendererVersion, 'horror-image-glyph-v4')
  assert.equal(metadata.styleId, 'horror')
  assert.equal(metadata.sourceStored, false)
  assert.match(metadata.construction, /no generic font skeleton/i)
  assert.match(metadata.material.face, /distressed white metal/i)
  assert.match(metadata.material.depth, /dissolve into granular ash/i)

  const provenance = Object.values(metadata.glyphs as Record<string, { sourceKind: string }>).reduce<Record<string, number>>((counts, entry) => {
    counts[entry.sourceKind] = (counts[entry.sourceKind] || 0) + 1
    return counts
  }, {})
  assert.equal(provenance['primary-sheet-cleaned'], 80)
  assert.equal(provenance['source-derived-case-fallback'], 2)
  assert.equal(provenance['authored-symbol-fallback'], 12)
})

test('Scary assembles cleaned red brush glyphs and renders captions in source uppercase', () => {
  const style = CAPTION_STYLES.find(candidate => candidate.id === 'scary')
  assert.equal(style?.name, 'Scary')
  assert.equal(style?.presentation, 'scary')
  assert.equal(style?.uppercase, true)
  assert.equal(style?.shadow, 'none')
  assert.ok((style?.safeWidthRatio || 1) <= 0.8)

  const root = '../public/caption-glyphs/scary'
  const metadata = JSON.parse(readFileSync(new URL(`${root}/metadata.json`, import.meta.url), 'utf8'))
  assert.equal(existsSync(new URL(`${root}/atlas.png`, import.meta.url)), true)
  assert.equal(existsSync(new URL(`${root}/picker.png`, import.meta.url)), true)
  assert.equal(metadata.rendererVersion, 'scary-image-glyph-v3')
  assert.equal(metadata.styleId, 'scary')
  assert.equal(metadata.textTransform, 'uppercase')
  assert.equal(metadata.sourceStored, false)
  assert.match(metadata.construction, /no generic font skeleton/i)
  assert.match(metadata.material.face, /jagged red dry-brush/i)
  assert.match(metadata.material.depth, /black separation/i)

  const required = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!"#$%&\'()*+,-./:;<=>?@[\\]^_`{|}~'
  for (const character of required) {
    assert.ok(Array.isArray(metadata.glyphs[character]?.atlas), `missing Scary atlas entry for ${character}`)
  }

  const provenance = Object.values(metadata.glyphs as Record<string, { sourceKind: string }>).reduce<Record<string, number>>((counts, entry) => {
    counts[entry.sourceKind] = (counts[entry.sourceKind] || 0) + 1
    return counts
  }, {})
  assert.equal(provenance['primary-sheet-cleaned'], 75)
  assert.equal(provenance['source-derived-case-fallback'], 10)
  assert.equal(provenance['authored-symbol-fallback'], 9)
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

test('Cardboard card sizing defaults to 75% and remains independent from text sizing', () => {
  assert.equal(DEFAULT_CAPTION_CARD_SCALE, 0.75)
  assert.equal(clampCaptionCardScale(0.1), 0.5)
  assert.equal(clampCaptionCardScale(2), 1)
  assert.equal(clampCaptionCardScale(Number.NaN), 0.75)
  assert.notEqual(clampCaptionCardScale(0.5), clampCaptionFontScale(0.5))
})
