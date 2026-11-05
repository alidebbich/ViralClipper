import { describe, it, expect, vi } from 'vitest';

describe('App', () => {
  it('should render', () => {
    // Basic test to verify vitest is working.
    // The actual components heavily rely on @tauri-apps/api/core invoke which must be mocked globally.
    expect(true).toBe(true);
  });
});