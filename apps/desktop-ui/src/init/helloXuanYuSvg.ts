/** 欢迎页手写 SVG 路径与描边时序（由 generated json 载入）。 */
import generatedIntro from './helloXuanYuPaths.generated.json';

type GeneratedIntroData = {
  viewBox: { w: number; h: number };
  path: string;
  draw: { delay: number; duration: number; easing: string };
  maskWidth: number;
  strokes: Array<Omit<IntroStroke, 'color'>>;
};

const introData = generatedIntro as GeneratedIntroData;

export const INTRO_VIEWBOX = introData.viewBox;
export const HELLO_XUANYU_PATH = introData.path;
export const INTRO_DRAW = introData.draw;

export type IntroStroke = {
  id: string;
  order?: number;
  letter: string;
  d: string;
  color: string;
  delay: number;
  duration: number;
  coreWidth: number;
  midWidth: number;
  glowWidth: number;
  sparkWidth: number;
};

export const RAINBOW_GRADIENT = {
  id: 'xy-rainbow-gradient',
  stops: [
    { offset: '0%', color: '#13f1e4' },
    { offset: '10%', color: '#15e7d6' },
    { offset: '16%', color: '#6cf046' },
    { offset: '24%', color: '#fff03a' },
    { offset: '33%', color: '#ff9b27' },
    { offset: '40%', color: '#ff5e91' },
    { offset: '52%', color: '#15e9e2' },
    { offset: '61%', color: '#62ef49' },
    { offset: '69%', color: '#fff034' },
    { offset: '77%', color: '#ff8b2d' },
    { offset: '86%', color: '#ff5594' },
    { offset: '100%', color: '#ff5c9c' },
  ],
};

const gradientStroke = `url(#${RAINBOW_GRADIENT.id})`;

export const HELLO_XUANYU_STROKES: IntroStroke[] = introData.strokes
  .slice()
  .sort((a, b) => (a.order ?? 0) - (b.order ?? 0))
  .map((stroke) => ({ ...stroke, color: gradientStroke }));

export const HELLO_XUANYU_STROKE: IntroStroke = HELLO_XUANYU_STROKES[0] ?? {
  ...introData.strokes[0],
  color: gradientStroke,
};

const lastStroke = HELLO_XUANYU_STROKES[HELLO_XUANYU_STROKES.length - 1];
export const INTRO_DURATION_MS = Math.round(
  ((lastStroke?.delay ?? INTRO_DRAW.delay) + (lastStroke?.duration ?? INTRO_DRAW.duration) + 0.2) * 1000,
);

export const RAINBOW_GRADIENT_BOUNDS = {
  x1: 34,
  x2: 1780,
};
