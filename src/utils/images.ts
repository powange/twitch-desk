// Image collée, glissée ou choisie → data URL réduite (côté le plus long : `max` px),
// pour garder la base légère
export async function imageToDataUrl(file: Blob, max = 256): Promise<string> {
  const url = URL.createObjectURL(file)
  try {
    const img = await new Promise<HTMLImageElement>((resolve, reject) => {
      const el = new Image()
      el.onload = () => resolve(el)
      el.onerror = () => reject(new Error('Image illisible'))
      el.src = url
    })
    const scale = Math.min(1, max / Math.max(img.naturalWidth, img.naturalHeight))
    const canvas = document.createElement('canvas')
    canvas.width = Math.max(1, Math.round(img.naturalWidth * scale))
    canvas.height = Math.max(1, Math.round(img.naturalHeight * scale))
    canvas.getContext('2d')?.drawImage(img, 0, 0, canvas.width, canvas.height)
    // WebP si le moteur sait l'encoder, sinon PNG
    const webp = canvas.toDataURL('image/webp', 0.85)
    return webp.startsWith('data:image/webp') ? webp : canvas.toDataURL('image/png')
  } finally {
    URL.revokeObjectURL(url)
  }
}

// Première image d'un collage ou d'un dépôt
export function firstImage(items: DataTransferItemList | FileList | null | undefined): File | null {
  if (!items) return null
  const files: (File | null)[] = items instanceof FileList
    ? Array.from(items)
    : Array.from(items).map(item => (item.kind === 'file' ? item.getAsFile() : null))
  return files.find(f => f?.type.startsWith('image/')) ?? null
}
