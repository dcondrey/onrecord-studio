import { useCallback, useRef } from 'react';
import { useAppStore } from '../store/useAppStore';

export function useAudioAlerts() {
  const isSyncing = useAppStore((s) => s.isSyncing);
  const audioContextRef = useRef<AudioContext | null>(null);

  /**
   * Initializes a native browser oscillator synthesis node locally.
   * Avoids asset packaging overhead by generating pure signal waves mathematically.
   */
  const synthesizeChime = useCallback((frequency: number, duration: number, type: OscillatorType = 'sine') => {
    try {
      if (!audioContextRef.current) {
        audioContextRef.current = new (window.AudioContext || (window as any).webkitAudioContext)();
      }
      
      const ctx = audioContextRef.current;
      if (ctx.state === 'suspended') {
        ctx.resume();
      }

      const osc = ctx.createOscillator();
      const gainNode = ctx.createGain();

      osc.type = type;
      osc.frequency.setValueAtTime(frequency, ctx.currentTime);
      
      // Clean linear envelope decay to avoid cracking audio waves
      gainNode.gain.setValueAtTime(0.15, ctx.currentTime);
      gainNode.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + duration);

      osc.connect(gainNode);
      gainNode.connect(ctx.destination);

      osc.start();
      osc.stop(ctx.currentTime + duration);
    } catch (e) {
      console.warn("Audio Context synthesis initialization blocked by system permissions:", e);
    }
  }, []);

  const triggerSuccessAlert = useCallback(() => {
    // Elegant arpeggio sequence indicating task success
    synthesizeChime(523.25, 0.1, 'sine'); // C5
    setTimeout(() => synthesizeChime(659.25, 0.2, 'sine'), 80); // E5
  }, [synthesizeChime]);

  const triggerFailureAlert = useCallback(() => {
    // Low double-pulse frequency shift flagging structural exceptions
    synthesizeChime(180.0, 0.15, 'sawtooth'); 
    setTimeout(() => synthesizeChime(150.0, 0.25, 'sawtooth'), 120);
  }, [synthesizeChime]);

  return {
    triggerSuccessAlert,
    triggerFailureAlert,
    synthesizeChime
  };
}
