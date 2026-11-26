// Opens a .mcpb the way Claude Desktop does, with yauzl and its strict checks, and reads every
// entry through; then requires the zip's comment to be exactly one mcpb signature block. Exits 1
// with yauzl's own message on anything Desktop would refuse.
//
// Usage: node zip-check.mjs <file.mcpb>
import { readFileSync } from 'node:fs'
import yauzl from 'yauzl'

const file = process.argv[2]
const fail = message => {
  console.error(`${file}: ${message}`)
  process.exit(1)
}

yauzl.open(file, { lazyEntries: true, validateEntrySizes: true, strictFileNames: true }, (err, zip) => {
  if (err) fail(err.message)
  zip.on('error', e => fail(e.message))
  zip.on('entry', entry => {
    if (entry.fileName.endsWith('/')) return zip.readEntry()
    zip.openReadStream(entry, (err, stream) => {
      if (err) fail(err.message)
      stream.on('error', e => fail(e.message))
      stream.on('end', () => zip.readEntry())
      stream.resume()
    })
  })
  zip.on('end', () => {
    // yauzl has checked the comment runs exactly to the end of the file; cp437 is one byte a char.
    const bytes = readFileSync(file)
    const comment = bytes.subarray(bytes.length - zip.comment.length)
    const head = Buffer.from('MCPB_SIG_V1')
    const foot = Buffer.from('MCPB_SIG_END')
    const signed =
      comment.length > head.length + 4 + foot.length &&
      comment.subarray(0, head.length).equals(head) &&
      comment.subarray(comment.length - foot.length).equals(foot) &&
      comment.readUInt32LE(head.length) === comment.length - head.length - 4 - foot.length
    if (!signed) fail('its zip comment is not one mcpb signature block, so it is not signed')
  })
  zip.readEntry()
})
