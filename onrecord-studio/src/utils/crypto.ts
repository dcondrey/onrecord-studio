export function isSha256Hex(value: string) {
  return value.length === 64 && Array.from(value).every((character) => {
    const code = character.toLocaleLowerCase().charCodeAt(0);
    return (code >= 48 && code <= 57) || (code >= 97 && code <= 102);
  });
}
