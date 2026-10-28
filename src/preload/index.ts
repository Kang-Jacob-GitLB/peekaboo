import { contextBridge, ipcRenderer } from 'electron'
import type { PeekabooApi } from '../shared/preview-api'
import { Channels, toPeekUrl } from '../shared/channels'

const api: PeekabooApi = {
  onShow: (cb) => {
    ipcRenderer.on(Channels.show, (_e, payload) => cb(payload))
  },
  onShowError: (cb) => {
    ipcRenderer.on(Channels.showError, (_e, err) => cb(err))
  },
  onHide: (cb) => {
    ipcRenderer.on(Channels.hide, () => cb())
  },
  readText: (path, maxBytes) => ipcRenderer.invoke(Channels.readText, path, maxBytes),
  listDir: (path, max) => ipcRenderer.invoke(Channels.listDir, path, max),
  fileUrl: (path) => toPeekUrl(path),
  rendered: (seq) => ipcRenderer.send(Channels.rendered, seq)
}

contextBridge.exposeInMainWorld('peekaboo', api)
