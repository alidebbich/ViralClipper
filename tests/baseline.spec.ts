import { test, expect } from '@playwright/test';

test('Baseline UI Capture', async ({ page }) => {
  // Mock Tauri invoke BEFORE navigation
  await page.addInitScript(() => {
    const win = window as any;
    win.__TAURI_INTERNALS__ = win.__TAURI_INTERNALS__ || {};
    win.__TAURI_INTERNALS__.transformCallback = function(callback: any, once?: boolean) {
      const id = Math.floor(Math.random() * 1000000);
      win['_' + id] = (arg: any) => {
        if (once) delete win['_' + id];
        return callback(arg);
      };
      return id;
    };
    win.__TAURI_INTERNALS__.ipc = function() {};
    win.__TAURI_INTERNALS__.unregisterListener = function() {};
    win.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    win.__TAURI_INTERNALS__.invoke = async (cmd: string, args: any) => {
      console.log('Mocking invoke:', cmd, args);
      if (cmd === 'get_projects') {
        return [{ id: 'proj1', name: 'Test Project', created_at: new Date().toISOString(), total_duration: 60, clip_count: 1 }];
      }
      if (cmd === 'get_clips') {
        return [{
          id: 'clip1', project_id: 'proj1', start_s: 0, end_s: 10, duration_s: 10,
          title: 'Test Clip', template_id: 'tiktok', status: 'rendered', hook_score: 9, viral_score: 8
        }];
      }
      if (cmd === 'list_templates') {
        return [
          { id: 'tiktok', name: 'TikTok', description: 'Pop-in karaoke', font_family: 'Arial', font_size: 24, font_color: '#ffffff', stroke_color: '#000000', stroke_width: 2, is_custom: false }
        ];
      }
      if (cmd === 'search_assets') return [];
      if (cmd === 'get_user_preference') return { value_json: JSON.stringify('test') };
      if (cmd === 'list_tts_voices_cmd') return ['ar-TN-HediNeural', 'en-US-GuyNeural'];
      if (typeof cmd === 'string' && cmd.includes('listen')) return 1;
      return null;
    };
  });

  page.on('console', msg => console.log('PAGE LOG:', msg.text()));
  page.on('pageerror', err => console.log('PAGE ERROR:', err.message));

  await page.goto('http://localhost:1420');

  // Wait for the app to load
  await page.waitForSelector('.main');

  // Tab 1: Queue (Default)
  await page.screenshot({ path: 'baseline/01_queue_empty.png' });

  // Tab 2: Projects
  await page.click('button:has-text("Projects")');
  await page.waitForTimeout(500);
  await page.screenshot({ path: 'baseline/02_projects_list.png' });

  // Tab 3: Create
  await page.click('button:has-text("Create")');
  await page.waitForTimeout(500);
  await page.screenshot({ path: 'baseline/03_create_tab.png' });

  // Tab 4: Templates
  await page.click('button:has-text("Templates")');
  await page.waitForTimeout(500);
  await page.screenshot({ path: 'baseline/04_templates_tab.png' });

  // Tab 5: Settings
  await page.click('button:has-text("Settings")');
  await page.waitForTimeout(500);
  await page.screenshot({ path: 'baseline/05_settings_tab.png' });
});
