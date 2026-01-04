# Documentation Index

## 📚 Main Documentation

### For Users
- **[README.md](../README.md)** - Main project documentation, setup, and features
- **[CLAUDE_MEMORY.md](../CLAUDE_MEMORY.md)** - Comprehensive technical reference for AI assistants

### For Developers
- **[TECHNICAL_DESIGN.md](./TECHNICAL_DESIGN.md)** - Architecture and system design
- **[HFT_PERFORMANCE.md](./HFT_PERFORMANCE.md)** - Performance characteristics and optimization

---

## 📖 User Guides

Located in `docs/guides/`:

- **[USER_GUIDE.md](./guides/USER_GUIDE.md)** - Detailed usage instructions
- **[POSITION_MANAGEMENT_GUIDE.md](./guides/POSITION_MANAGEMENT_GUIDE.md)** - Position tracking and management
- **[REFACTORING_PLAN.md](./guides/REFACTORING_PLAN.md)** - Code improvement roadmap (350+ lines)
- **[REFACTORING_QUICKSTART.md](./guides/REFACTORING_QUICKSTART.md)** - Quick start for refactoring
- **[REFACTORING_SUMMARY.md](./guides/REFACTORING_SUMMARY.md)** - Phase 1 refactoring summary

---

## 🔧 Bug Fixes & Solutions

Located in `docs/fixes/`:

### Critical Fixes
1. **[INFINITE_LOOP_COMPLETE_SUMMARY.md](./fixes/INFINITE_LOOP_COMPLETE_SUMMARY.md)** - Infinite retry loop prevention
2. **[ORPHANED_POSITION_FIX.md](./fixes/ORPHANED_POSITION_FIX.md)** - Auto-recreates exit orders for orphaned positions
3. **[QUANTITY_MISMATCH_FIX.md](./fixes/QUANTITY_MISMATCH_FIX.md)** - Handles partial fills and quantity mismatches
4. **[POSITION_NOT_FOUND_FIX.md](./fixes/POSITION_NOT_FOUND_FIX.md)** - Cleans up positions not on exchange
5. **[RETRY_ON_ERROR_FIX.md](./fixes/RETRY_ON_ERROR_FIX.md)** - Smart retry with fresh verification

### Position Management
- **[RESTART_HANDLING_YES.md](./fixes/RESTART_HANDLING_YES.md)** - How restarts handle existing positions
- **[RESTART_POSITION_HANDLING.md](./fixes/RESTART_POSITION_HANDLING.md)** - Detailed restart behavior
- **[ORPHANED_POSITION_QUICKFIX.md](./fixes/ORPHANED_POSITION_QUICKFIX.md)** - Quick reference for orphan fixes

### Trading Logic
- **[TAKE_PROFIT_STOP_LOSS_COMPLETE.md](./fixes/TAKE_PROFIT_STOP_LOSS_COMPLETE.md)** - TP/SL implementation
- **[SELL_LOGIC_ANALYSIS.md](./fixes/SELL_LOGIC_ANALYSIS.md)** - Sell order logic analysis
- **[NO_TRADE_COOLDOWN.md](./fixes/NO_TRADE_COOLDOWN.md)** - Trade cooldown implementation

### Order Management
- **[ORDER_MINIMUM_FIX.md](./fixes/ORDER_MINIMUM_FIX.md)** - Order size validation
- **[ORDER_VALIDATION_TESTS.md](./fixes/ORDER_VALIDATION_TESTS.md)** - Order validation tests

### Testing
- **[TESTS.md](./fixes/TESTS.md)** - Testing documentation

---

## 🚨 Urgent Reference

For immediate issues, check:

1. **[INFINITE_LOOP_FIX_URGENT.md](./fixes/INFINITE_LOOP_FIX_URGENT.md)** - Stop application immediately if stuck in loop
2. **[CLAUDE_MEMORY.md](../CLAUDE_MEMORY.md)** - Section "⚠️ Known Caveats & Gotchas"
3. **[README.md](../README.md)** - Section "🐛 Troubleshooting"

---

## 📊 Quick Links by Topic

### Setup & Configuration
- [README.md](../README.md) - Installation & Configuration
- [.env.example](../.env.example) - Environment variables template
- [config.yaml](../config.yaml) - Trading parameters

### Architecture & Design
- [TECHNICAL_DESIGN.md](./TECHNICAL_DESIGN.md) - System architecture
- [CLAUDE_MEMORY.md](../CLAUDE_MEMORY.md) - Implementation details
- [HFT_PERFORMANCE.md](./HFT_PERFORMANCE.md) - Performance metrics

### Common Issues
- **"Insufficient balance"** → [QUANTITY_MISMATCH_FIX.md](./fixes/QUANTITY_MISMATCH_FIX.md)
- **"Position not found"** → [POSITION_NOT_FOUND_FIX.md](./fixes/POSITION_NOT_FOUND_FIX.md)
- **"Rate limit exceeded"** → [INFINITE_LOOP_COMPLETE_SUMMARY.md](./fixes/INFINITE_LOOP_COMPLETE_SUMMARY.md)
- **Positions without exits** → [ORPHANED_POSITION_FIX.md](./fixes/ORPHANED_POSITION_FIX.md)
- **Restart behavior** → [RESTART_HANDLING_YES.md](./fixes/RESTART_HANDLING_YES.md)

### Development
- [REFACTORING_PLAN.md](./guides/REFACTORING_PLAN.md) - Code improvement roadmap
- [CLAUDE_MEMORY.md](../CLAUDE_MEMORY.md) - Development patterns and caveats
- [TESTS.md](./fixes/TESTS.md) - Testing guide

---

## 📁 File Organization

```
rust-autohedge/
├── README.md                     # Main documentation
├── CLAUDE_MEMORY.md              # AI assistant reference
├── .env.example                  # Environment template
├── config.yaml                   # Trading configuration
├── docs/
│   ├── INDEX.md                  # This file
│   ├── TECHNICAL_DESIGN.md       # Architecture
│   ├── HFT_PERFORMANCE.md        # Performance docs
│   ├── guides/                   # User and developer guides
│   │   ├── USER_GUIDE.md
│   │   ├── POSITION_MANAGEMENT_GUIDE.md
│   │   ├── REFACTORING_PLAN.md
│   │   ├── REFACTORING_QUICKSTART.md
│   │   └── REFACTORING_SUMMARY.md
│   └── fixes/                    # Bug fixes and solutions
│       ├── INFINITE_LOOP_COMPLETE_SUMMARY.md
│       ├── ORPHANED_POSITION_FIX.md
│       ├── QUANTITY_MISMATCH_FIX.md
│       ├── POSITION_NOT_FOUND_FIX.md
│       ├── RETRY_ON_ERROR_FIX.md
│       └── ... (more fix docs)
└── src/                          # Source code
```

---

## 🎯 Recommended Reading Order

### For New Users
1. [README.md](../README.md) - Overview and setup
2. [.env.example](../.env.example) - Configure environment
3. Start application and test
4. [USER_GUIDE.md](./guides/USER_GUIDE.md) - Detailed usage

### For Developers
1. [README.md](../README.md) - Project overview
2. [CLAUDE_MEMORY.md](../CLAUDE_MEMORY.md) - Complete technical reference
3. [TECHNICAL_DESIGN.md](./TECHNICAL_DESIGN.md) - Architecture
4. [REFACTORING_PLAN.md](./guides/REFACTORING_PLAN.md) - Improvement roadmap

### For AI Assistants
1. **[CLAUDE_MEMORY.md](../CLAUDE_MEMORY.md)** - Start here! Complete implementation reference
2. [README.md](../README.md) - User-facing documentation
3. Specific fix docs in `docs/fixes/` as needed

### For Troubleshooting
1. [README.md](../README.md) - Section "🐛 Troubleshooting"
2. Find your issue in **"📊 Quick Links by Topic"** above
3. [CLAUDE_MEMORY.md](../CLAUDE_MEMORY.md) - Section "⚠️ Known Caveats"

---

## 🔄 Document Maintenance

### When to Update

- **README.md**: New features, setup changes, API changes
- **CLAUDE_MEMORY.md**: Implementation changes, new caveats, critical patterns
- **Fix docs**: When bugs are fixed (create new doc in `docs/fixes/`)
- **This INDEX.md**: When adding/removing/moving documentation

### Document Templates

**Bug Fix Document** (save to `docs/fixes/`):
```markdown
# [Bug Name] Fix

## Problem
Description of the issue

## Root Cause
What caused it

## Solution
How it was fixed

## Testing
How to verify the fix

## Files Changed
List of modified files
```

---

**Last Updated**: January 3, 2026  
**Total Docs**: 25+ files  
**Status**: Organized ✅

