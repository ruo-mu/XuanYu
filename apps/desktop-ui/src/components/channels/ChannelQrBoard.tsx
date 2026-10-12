/**
 * 频道登录二维码：把深链或图片地址画到 canvas，供扫码绑定。
 */

import React, { useEffect, useRef, useState } from 'react';
import QRCode from 'qrcode';
import { useLocaleText } from '../../i18n';

/** 把微信 iLink 返回的深链编码成可扫描二维码（不是图片 URL）。 */
export function ChannelQrBoard({
  payload,
  loading,
  statusText,
  size = 132,
}: {
  payload?: string | null;
  loading?: boolean;
  statusText?: string;
  size?: number;
}) {
  const lt = useLocaleText();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [error, setError] = useState<string | null>(null);
  const looksLikeImage =
    !!payload &&
    (payload.startsWith('data:image') ||
      /\.(png|jpe?g|gif|webp)(\?|$)/i.test(payload) ||
      /\/qrcode|qr_code|showqrcode/i.test(payload));

  useEffect(() => {
    let cancelled = false;
    const canvas = canvasRef.current;
    if (!canvas || !payload || looksLikeImage) {
      setError(null);
      return;
    }
    void QRCode.toCanvas(canvas, payload, {
      width: size,
      margin: 2,
      color: { dark: '#111827', light: '#ffffff' },
      errorCorrectionLevel: 'M',
    })
      .then(() => {
        if (!cancelled) setError(null);
      })
      .catch((err) => {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [payload, size, looksLikeImage]);

  return (
    <div className="xy-ch-qr-board">
      <div className="xy-ch-qr-frame">
        {payload ? (
          looksLikeImage ? (
            <img src={payload} width={size} height={size} alt={lt('登录二维码', 'Login QR')} />
          ) : (
            <canvas ref={canvasRef} width={size} height={size} aria-label={lt('登录二维码', 'Login QR')} />
          )
        ) : (
          <div className="xy-ch-qr-skeleton" style={{ width: size, height: size }}>
            <span className={loading ? 'is-pulse' : undefined}>
              {loading ? lt('出码中', 'Generating') : lt('等待出码', 'Waiting')}
            </span>
          </div>
        )}
      </div>
      {statusText ? <p className="xy-ch-qr-status">{statusText}</p> : null}
      {error ? <p className="xy-ch-qr-error">{error}</p> : null}
    </div>
  );
}
