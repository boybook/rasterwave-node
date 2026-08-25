'use strict'

const native = require('./native.js')

const SstvMode = Object.freeze({
  Robot8Bw: 'robot8Bw', Robot12Bw: 'robot12Bw', Robot24Bw: 'robot24Bw', Robot36Bw: 'robot36Bw',
  Robot12: 'robot12', Robot24: 'robot24', Robot36: 'robot36', Robot72: 'robot72',
  Martin1: 'martin1', Martin2: 'martin2', Martin3: 'martin3', Martin4: 'martin4',
  Scottie1: 'scottie1', Scottie2: 'scottie2', Scottie3: 'scottie3', Scottie4: 'scottie4', ScottieDx: 'scottieDx',
  Pd50: 'pd50', Pd90: 'pd90', Pd120: 'pd120', Pd160: 'pd160', Pd180: 'pd180', Pd240: 'pd240', Pd290: 'pd290',
  WraaseSc2_30: 'wraaseSc2_30', WraaseSc2_60: 'wraaseSc2_60', WraaseSc2_120: 'wraaseSc2_120', WraaseSc2_180: 'wraaseSc2_180',
  Pasokon3: 'pasokon3', Pasokon5: 'pasokon5', Pasokon7: 'pasokon7',
})

const FaxIoc = Object.freeze({ Ioc288: 'ioc288', Ioc576: 'ioc576' })
const FaxPolarity = Object.freeze({ Normal: 'normal', Inverted: 'inverted' })

module.exports.SstvDecoder = native.SstvDecoder
module.exports.SstvEncoder = native.SstvEncoder
module.exports.FaxDecoder = native.FaxDecoder
module.exports.FaxEncoder = native.FaxEncoder
module.exports.correctFaxPaper = native.correctFaxPaper
module.exports.SstvMode = SstvMode
module.exports.FaxIoc = FaxIoc
module.exports.FaxPolarity = FaxPolarity
module.exports.sstvModes = native.sstvModes
