// UniFFI wire-format serialization helpers.
//
// UniFFI serializes compound types into RustBuffer using a simple
// binary format: each value is preceded by a length or tag, all
// integers are big-endian. These helpers convert between JS values
// and Uint8Array buffers matching that format.

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/**
 * Lower a JS string into a Uint8Array matching UniFFI's string wire format:
 * 4-byte big-endian Int32 byte length, followed by UTF-8 bytes.
 */
export function lowerString(s) {
  const encoded = encoder.encode(s);
  const buf = new Uint8Array(4 + encoded.length);
  new DataView(buf.buffer).setInt32(0, encoded.length, false);
  buf.set(encoded, 4);
  return buf;
}

/**
 * Lift a Uint8Array (from a RustBuffer return) into a JS string.
 * Reads: 4-byte big-endian Int32 byte length, then that many UTF-8 bytes.
 */
export function liftString(buf) {
  const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  const len = view.getInt32(0, false);
  return decoder.decode(buf.slice(4, 4 + len));
}

/**
 * Lift a UniFFI error enum from a Uint8Array (from RustCallStatus.errorBuf).
 * Reads: 4-byte big-endian Int32 variant index, then variant fields.
 * Returns { variant: number, ...fields }.
 *
 * For ArithmeticError::DivisionByZero { reason: String }:
 *   variant=1, reason=liftString(remaining bytes)
 */
export function liftArithmeticError(buf) {
  const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  const variant = view.getInt32(0, false);
  const reason = liftString(buf.slice(4));
  return { variant, reason };
}
