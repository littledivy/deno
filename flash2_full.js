const { ops } = Deno.core

const {
  op_flash_try_write_str, op_flash_start, op_flash_try_write_status_str,
  op_flash_try_write, op_flash_write, op_flash_set_date
} = ops

const Types = {
  Default: 'default',
  Buffer: 'typedarray'
}

class Response {
  status = 200
  contentType = 'text/plain;charset=UTF-8'
  statusMessage = ''
  type = Types.Default
  body = null
  headerList = []
  urlList = []

  constructor (body = null, init = undefined) {
    if (typeof body === "string") {
      this.body = body
      return
    }
    if (body.constructor.name === 'Uint8Array') {
      this.body = body
      this.type = Types.Buffer
      return
    }
  }

  url () {
    if (this.urlList.length == 0) return null;
    return this.urlList[this.urlList.length - 1];
  }
}

function createResponseString (res) {
  const { status, statusMessage, body, contentType } = res
  return `HTTP/1.1 ${status} ${statusMessage} \r\nDate: ${now}\r\nContent-Length: ${13}\r\nContent-Type: ${contentType}\r\n\r\n${body}`
}

Deno.serve = async (fetch, options) => {
  const mode = fetch.constructor.name === 'AsyncFunction' ? 'async': 'sync';
  const argLen = fetch.length;
  if (mode === 'sync') {
    // sync handler
    if (argLen === 0) {
      // fast path - sync handler with no request argument - 400k rps
      await op_flash_start((rid) => {
        const res = fetch()
        if (res.type === Types.Default) {
          //op_flash_try_write_str(rid, createResponseString(res))
          // todo: return codes
          //console.log(res.body)
          op_flash_try_write_status_str(rid, res.status, res.body)
          return
        }
        if (res.type === Types.Buffer) {
          const nwritten = op_flash_try_write(rid, res.body);
          if (nwritten < res.body.byteLength) {
            op_flash_write(rid, res.body);
          }
        }
      })
      return
    }
    // slow path - we pass a request object
    await op_flash_start((rid) => {
      const request = fromFlashRequest(0, rid, null, nop, nop, nop);
      const res = fetch(request)
      if (res.type === Types.Default) {
        op_flash_try_write_str(rid, createResponseString(res))
      }
    })
    return
  }
  // async handler
}

const timer = setInterval(() => {
  op_flash_set_date((new Date()).toUTCString())
}, 1000)

Deno.serve(() => new Response('Hello, World!'))
//const encoder = new TextEncoder()
//const u8 = encoder.encode('Hello, World!')
//Deno.serve(() => new Response(u8))
