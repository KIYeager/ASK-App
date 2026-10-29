// Läuft im unsichtbaren Fenster von visual.js. pdf.worker.min.js ist als Skript geladen, damit pdf.js
// ohne eigenen Worker im Fenster selbst arbeitet (Worker aus file:// sind gesperrt).

function base64ToBytes(b64) {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

// PDF-Seiten (1-basiert) als PNG-data-URLs mit höchstens width Pixel Breite
window.renderPdf = async (b64, pages, width) => {
  const doc = await window.pdfjsLib.getDocument({ data: base64ToBytes(b64), isEvalSupported: false }).promise;
  const images = [];
  for (const n of pages.filter((p) => p >= 1 && p <= doc.numPages)) {
    const page = await doc.getPage(n);
    const base = page.getViewport({ scale: 1 });
    const viewport = page.getViewport({ scale: Math.min(width / base.width, 3) });
    const canvas = document.createElement('canvas');
    canvas.width = Math.round(viewport.width);
    canvas.height = Math.round(viewport.height);
    const ctx = canvas.getContext('2d');
    ctx.fillStyle = '#fff';
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    await page.render({ canvasContext: ctx, viewport }).promise;
    images.push({ page: n, url: canvas.toDataURL('image/png') });
  }
  const count = doc.numPages;
  await doc.destroy();
  return { count, images };
};

// Bild (data-URL, auch SVG, GIF, WebP) als PNG mit höchstens max Pixel Kantenlänge, weisser Hintergrund
window.renderImage = (dataUrl, max) => new Promise((resolve, reject) => {
  const img = new Image();
  img.onload = () => {
    const w = img.naturalWidth || 800;
    const h = img.naturalHeight || 600;
    const scale = Math.min(1, max / Math.max(w, h));
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(w * scale));
    canvas.height = Math.max(1, Math.round(h * scale));
    const ctx = canvas.getContext('2d');
    ctx.fillStyle = '#fff';
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
    resolve({ width: w, height: h, url: canvas.toDataURL('image/png') });
  };
  img.onerror = () => reject(new Error('Bild lässt sich nicht öffnen.'));
  img.src = dataUrl;
});
