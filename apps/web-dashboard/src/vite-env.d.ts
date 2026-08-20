/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_HL_API_ORIGIN?: string
  readonly VITE_HL_API_BEARER?: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
