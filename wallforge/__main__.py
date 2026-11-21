import sys

if "--daemon" in sys.argv:  # light path: no GTK/Adw imports
    from .daemon import main
else:
    from .app import main

sys.exit(main())
