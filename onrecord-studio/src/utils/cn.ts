import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/**
 * Dynamically combines conditional class strings and safely resolves 
 * conflicting Tailwind styles natively before rendering components.
 */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
