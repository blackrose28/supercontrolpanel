# Mobile Viewport Fixes for iPhone 14 Pro Max (430x932)

## Issues Fixed
- Buttons were appearing offscreen due to horizontal overflow
- Navigation toolbar wasn't wrapping properly on narrow screens
- Action buttons were overflowing the viewport width

## Key Changes

### 1. Horizontal Overflow Prevention
- Added `overflow-x: hidden` to html, body, .app, .views, and .view
- Set `max-width: 100vw` on .app container
- Applied `max-width: 100%` to all elements in mobile breakpoint

### 2. Toolbar Navigation (< 768px)
- Reduced padding: `6px 8px` (was `8px`)
- Reduced gap: `4px` (was `6px`)
- Smaller font: `12px` (was `13px`)
- Added `overflow-x: hidden` to prevent horizontal scroll
- Version badge: `10px` font, `2px 6px` padding

### 3. Action Buttons
- Set `width: 100%` and `box-sizing: border-box`
- Buttons flex at `calc(50% - 3px)` to fit two per row
- On screens < 480px, buttons stack at `100%` width
- Added `min-width: 0` to prevent flex overflow

### 4. Form Inputs
- All inputs/selects: `width: 100%` with `box-sizing: border-box`
- Form grids collapse to single column
- Inline input actions stack vertically

### 5. Search Bars & Toolbars
- Server search bar: flex layout with input taking remaining space
- All toolbars: `width: 100%` with proper box-sizing

### 6. Modals
- Max width: `calc(100vw - 16px)` to account for padding
- Modal actions stack vertically on mobile
- All buttons: `width: 100%`

### 7. Touch Targets (Touch Devices)
- Buttons: `min-height: 44px` (Apple HIG standard)
- Inputs/selects: `min-height: 44px`
- Nav buttons: `min-height: 40px`
- Link buttons: `min-height: 40px`

## Breakpoints
- **768px**: Main mobile breakpoint (tablets and phones)
- **480px**: Extra small screens (older/smaller phones)
- **Touch devices**: `(hover: none) and (pointer: coarse)`
- **Landscape**: Special handling for mobile landscape orientation

## Testing Recommendations
1. Test on iPhone 14 Pro Max (430x932)
2. Test on smaller devices (iPhone SE: 375x667)
3. Test landscape orientation
4. Verify all buttons are visible and tappable
5. Check that no horizontal scrolling occurs
6. Verify forms are usable with on-screen keyboard

## Files Modified
- `src/styles.css`: Added comprehensive mobile viewport styles (430 lines)
