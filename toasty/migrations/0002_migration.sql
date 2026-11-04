CREATE TABLE "asks" (
    "id" BLOB NOT NULL,
    "question" TEXT NOT NULL,
    "state" TEXT NOT NULL,
    "ranking" TEXT,
    "explanation" TEXT NOT NULL,
    "explanation_complete" BOOLEAN NOT NULL,
    "failure" TEXT,
    "created_at" INTEGER NOT NULL,
    "updated_at" INTEGER NOT NULL,
    PRIMARY KEY ("id")
);
