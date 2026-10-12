/**
 * 工作台静态光晕背景：多层斑点与 blob，无动画。
 */

import React from 'react';

/** 静态 soft mesh 背景：光晕 + 斑点，无动画、无线条 */
export default function AmbientBackground() {
  return (
    <div className="xy-ambient-layer" aria-hidden>
      <div className="xy-ambient-blob xy-ambient-blob--1" />
      <div className="xy-ambient-blob xy-ambient-blob--2" />
      <div className="xy-ambient-blob xy-ambient-blob--3" />
      <div className="xy-ambient-dot xy-ambient-dot--1" />
      <div className="xy-ambient-dot xy-ambient-dot--2" />
      <div className="xy-ambient-dot xy-ambient-dot--3" />
      <div className="xy-ambient-dot xy-ambient-dot--4" />
      <div className="xy-ambient-dot xy-ambient-dot--5" />
      <div className="xy-ambient-dot xy-ambient-dot--6" />
      <div className="xy-ambient-dot xy-ambient-dot--7" />
      <div className="xy-ambient-dot xy-ambient-dot--8" />
      <div className="xy-ambient-dot xy-ambient-dot--9" />
      <div className="xy-ambient-dot xy-ambient-dot--10" />
      <div className="xy-ambient-dot xy-ambient-dot--11" />
      <div className="xy-ambient-dot xy-ambient-dot--12" />
      <div className="xy-ambient-dot xy-ambient-dot--13" />
      <div className="xy-ambient-dot xy-ambient-dot--14" />
      <div className="xy-ambient-dot xy-ambient-dot--15" />
      <div className="xy-ambient-speck xy-ambient-speck--1" />
      <div className="xy-ambient-speck xy-ambient-speck--2" />
      <div className="xy-ambient-speck xy-ambient-speck--3" />
      <div className="xy-ambient-speck xy-ambient-speck--4" />
      <div className="xy-ambient-speck xy-ambient-speck--5" />
      <div className="xy-ambient-speck xy-ambient-speck--6" />
      <div className="xy-ambient-speck xy-ambient-speck--7" />
      <div className="xy-ambient-speck xy-ambient-speck--8" />
      <div className="xy-ambient-speck xy-ambient-speck--9" />
      <div className="xy-ambient-speck xy-ambient-speck--10" />
      <div className="xy-ambient-speck xy-ambient-speck--11" />
      <div className="xy-ambient-speck xy-ambient-speck--12" />
    </div>
  );
}
