import { useCallback, useEffect, useRef } from 'react';

export function useAudioAlerts(enabled: boolean, volume = 0.12) {
  const audioContextRef = useRef<AudioContext | null>(null);
  const timersRef = useRef(new Set<number>());
  const oscillatorsRef = useRef(new Set<OscillatorNode>());

  useEffect(() => () => {
    for (const timer of timersRef.current) window.clearTimeout(timer);
    timersRef.current.clear();
    for (const oscillator of oscillatorsRef.current) {
      try { oscillator.stop(); } catch { /* already stopped */ }
      oscillator.disconnect();
    }
    oscillatorsRef.current.clear();
    const context = audioContextRef.current;
    audioContextRef.current = null;
    if (context && context.state !== 'closed') void context.close();
  }, []);

  /**
   * Initializes a native browser oscillator synthesis node locally.
   * Avoids asset packaging overhead by generating pure signal waves mathematically.
   */
  const synthesizeChime = useCallback((frequency: number, duration: number, type: OscillatorType = 'sine') => {
    if (!enabled || !Number.isFinite(frequency) || frequency <= 0 || duration <= 0) return;
    try {
      if (!audioContextRef.current) {
        const AudioContextConstructor = window.AudioContext ??
          (window as Window & { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
        if (!AudioContextConstructor) {
          throw new Error('Web Audio is unavailable');
        }
        audioContextRef.current = new AudioContextConstructor();
      }
      
      const ctx = audioContextRef.current;
      if (ctx.state === 'suspended') void ctx.resume();

      const osc = ctx.createOscillator();
      const gainNode = ctx.createGain();

      osc.type = type;
      osc.frequency.setValueAtTime(frequency, ctx.currentTime);
      
      // Clean linear envelope decay to avoid cracking audio waves
      gainNode.gain.setValueAtTime(Math.min(Math.max(volume, 0.001), 0.25), ctx.currentTime);
      gainNode.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + duration);

      osc.connect(gainNode);
      gainNode.connect(ctx.destination);

      osc.start();
      osc.stop(ctx.currentTime + duration);
      oscillatorsRef.current.add(osc);
      osc.addEventListener('ended', () => {
        oscillatorsRef.current.delete(osc);
        osc.disconnect();
        gainNode.disconnect();
      }, { once: true });
    } catch (e) {
      console.warn("Audio Context synthesis initialization blocked by system permissions:", e);
    }
  }, [enabled, volume]);

  const schedule = useCallback((callback: () => void, delay: number) => {
    const timer = window.setTimeout(() => {
      timersRef.current.delete(timer);
      callback();
    }, delay);
    timersRef.current.add(timer);
  }, []);

  const triggerSuccessAlert = useCallback(() => {
    // Elegant arpeggio sequence indicating task success
    synthesizeChime(523.25, 0.1, 'sine'); // C5
    schedule(() => synthesizeChime(659.25, 0.2, 'sine'), 80); // E5
  }, [schedule, synthesizeChime]);

  const triggerFailureAlert = useCallback(() => {
    // Low double-pulse frequency shift flagging structural exceptions
    synthesizeChime(180.0, 0.15, 'sawtooth'); 
    schedule(() => synthesizeChime(150.0, 0.25, 'sawtooth'), 120);
  }, [schedule, synthesizeChime]);

  return {
    triggerSuccessAlert,
    triggerFailureAlert,
    synthesizeChime
  };
}
